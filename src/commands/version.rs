use crate::{cli::Options, config::Config, json::Value, manifest, semver::Version};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    if !output.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn lifecycle(root: &Path, manifest: &Value, event: &str, version: &str) -> Result<()> {
    let Some(script) = manifest.get("scripts").and_then(|s| s.str(event)) else {
        return Ok(());
    };
    let mut paths: Vec<_> = root
        .ancestors()
        .map(|p| p.join("node_modules/.bin"))
        .collect();
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    #[cfg(not(windows))]
    let mut command = {
        let mut c = Command::new("sh");
        c.arg("-c");
        c
    };
    #[cfg(windows)]
    let mut command = {
        let mut c = Command::new("cmd");
        c.args(["/d", "/s", "/c"]);
        c
    };
    let status = command
        .arg(script)
        .current_dir(root)
        .env("PATH", std::env::join_paths(paths)?)
        .env("npm_lifecycle_event", event)
        .env("npm_package_version", version)
        .env("npm_package_json", root.join("package.json"))
        .status()?;
    if !status.success() {
        bail!("{event} script failed ({status})");
    }
    Ok(())
}

fn update_files(root: &Path, version: &str) -> Result<()> {
    let mut updates = Vec::new();
    for name in ["package.json", "tnpm-lock.json"] {
        let path = root.join(name);
        let old = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && name != "package.json" => {
                continue;
            }
            Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
        };
        let mut value =
            Value::parse(&old).with_context(|| format!("could not parse {}", path.display()))?;
        if !matches!(value, Value::Object(_)) {
            bail!("{} must contain a JSON object", path.display());
        }
        value.set("version", Value::from(version));
        if let Some(root) = value.get_mut("packages").and_then(|p| p.get_mut("")) {
            root.set("version", Value::from(version));
        }
        updates.push((path, old, format!("{}\n", value.pretty())));
    }
    // Validate every file before changing any of them. Roll back our own writes on I/O failure.
    for (index, (path, _, new)) in updates.iter().enumerate() {
        if let Err(error) = fs::write(path, new) {
            for (path, old, _) in &updates[..=index] {
                let _ = fs::write(path, old);
            }
            return Err(error).with_context(|| format!("could not write {}", path.display()));
        }
    }
    Ok(())
}

pub fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    let manifest = manifest::read(root)?;
    let Some(release) = options.arguments.first() else {
        let mut versions = Value::object();
        if let Some(manifest) = manifest
            && let Some(version) = manifest.str("version")
        {
            versions.set(
                manifest.str("name").unwrap_or("package"),
                Value::from(version),
            );
        }
        versions.set("tnpm", Value::from(env!("CARGO_PKG_VERSION")));
        println!("{}", versions.pretty());
        return Ok(());
    };
    let manifest = manifest.context("package.json not found")?;
    let current = manifest.str("version").unwrap_or("0.0.0");
    let old = Version::parse(current).context("invalid version in package.json")?;
    let target = if release == "from-git" {
        let tag = git(root, &["describe", "--tags", "--abbrev=0"])?;
        if Version::parse(&tag).is_none() {
            bail!("latest Git tag is not a valid version: {tag}");
        }
        tag
    } else {
        release.clone()
    };
    let version = old
        .bump(
            &target,
            options
                .preid
                .as_deref()
                .or_else(|| config.get("preid"))
                .unwrap_or(""),
        )?
        .to_string();
    if current.trim_start_matches('v') == version
        && !options.allow_same_version
        && !config.enabled("allow-same-version")
    {
        bail!("version not changed (use --allow-same-version to allow this)");
    }
    let tag = format!(
        "{}{version}",
        config.get("tag-version-prefix").unwrap_or("v")
    );
    let do_git = !options.no_git_tag
        && config.enabled("git-tag-version")
        && git(root, &["rev-parse", "--is-inside-work-tree"]).is_ok();
    if do_git {
        if !git(root, &["status", "--porcelain"])?.is_empty() {
            bail!(
                "Git working directory is not clean; use --no-git-tag-version to skip Git operations"
            );
        }
        if git(
            root,
            &["rev-parse", "--verify", &format!("refs/tags/{tag}")],
        )
        .is_ok()
        {
            bail!("Git tag already exists: {tag}");
        }
    }
    let scripts = !options.ignore_scripts && !config.enabled("ignore-scripts");
    if scripts {
        lifecycle(root, &manifest, "preversion", current)?;
    }
    update_files(root, &version)?;
    if scripts {
        lifecycle(root, &manifest, "version", &version)?;
    }
    if do_git {
        let mut args = vec!["add", "--", "package.json"];
        if root.join("tnpm-lock.json").is_file() {
            args.push("tnpm-lock.json");
        }
        git(root, &args)?;
        git(root, &["commit", "-m", &tag])?;
        git(root, &["tag", "-a", &tag, "-m", &tag])?;
    }
    if scripts {
        lifecycle(root, &manifest, "postversion", &version)?;
    }
    if options.json {
        println!("{}", Value::from(version.as_str()).pretty());
    } else {
        println!("v{version}");
    }
    Ok(())
}
