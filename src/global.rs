//! User-scoped global installation with ownership-tracked executable links.
use crate::{
    cli::Options, commands, config::Config, fs::write_json, install, json::Value, manifest, scripts,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn safe_directory(path: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "refusing symlinked or non-directory global installation directory: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn run(options: &Options) -> Result<()> {
    ensure!(
        !options.no_save,
        "global installation requires its ownership manifest; --no-save is not supported with --global"
    );
    ensure!(
        !options.workspaces && options.workspace.is_empty(),
        "global and workspace options cannot be combined"
    );
    ensure!(
        matches!(
            options.command.as_str(),
            "install" | "uninstall" | "update" | "outdated" | "ls"
        ),
        "--global supports install, uninstall, update, outdated and ls"
    );
    let home = dirs::home_dir().context("home directory not found")?;
    safe_directory(&home.join(".tnpm"))?;
    let config = Config::load(&home.join(".tnpm"), options)?;
    let prefix = options
        .prefix
        .clone()
        .unwrap_or_else(|| PathBuf::from(config.get("prefix").unwrap()));
    let prefix = if prefix.is_absolute() {
        prefix
    } else {
        std::env::current_dir()?.join(prefix)
    };
    safe_directory(&prefix)?;
    let root = if cfg!(windows) {
        prefix.clone()
    } else {
        prefix.join("lib")
    };
    safe_directory(&root)?;
    fs::create_dir_all(&root)?;
    let manifest_path = root.join("package.json");
    ensure!(
        !fs::symlink_metadata(&manifest_path).is_ok_and(|m| m.file_type().is_symlink()),
        "global manifest must not be a symlink"
    );
    if !manifest_path.exists() {
        ensure_unmanaged_packages_absent(&root, None)?;
        write_json(
            &manifest_path,
            &Value::parse(r#"{"name":"tnpm-global","private":true}"#)?,
        )?;
    } else {
        let data = manifest::read(&root)?.context("global manifest missing")?;
        ensure!(
            data.str("name") == Some("tnpm-global")
                && data.get("private") == Some(&Value::Bool(true)),
            "refusing to modify an unrelated global manifest: {}",
            manifest_path.display()
        );
        if options.command != "ls" && options.command != "outdated" {
            ensure_unmanaged_packages_absent(
                &root,
                crate::lockfile::Lockfile::read(&root)?.as_ref(),
            )?;
        }
    }
    let mut local = options.clone();
    local.global = false;
    local.prefix = Some(root.clone());
    // Relative file arguments refer to the caller, not the global prefix.
    if local.command == "install" {
        for arg in &mut local.arguments {
            if let Ok((name, Some(selector))) = crate::spec::package_spec(arg)
                && let Some(path) = selector.strip_prefix("file:")
            {
                *arg = format!("{name}@file:{}", fs::canonicalize(path)?.display());
                continue;
            }
            let path = arg.strip_prefix("file:").or_else(|| {
                (arg.starts_with("./") || arg.starts_with("../") || Path::new(arg).is_absolute())
                    .then_some(arg.as_str())
            });
            if let Some(path) = path {
                *arg = format!("file:{}", fs::canonicalize(path)?.display());
            }
        }
    }
    if local.command == "ls" {
        return commands::packages::list(&root, &local);
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(install::run(&root, &local, &config))?;
    if local.command != "outdated" && !local.lock_only {
        sync_bins(&prefix, &root)?;
    }
    Ok(())
}

fn ensure_unmanaged_packages_absent(
    root: &Path,
    lock: Option<&crate::lockfile::Lockfile>,
) -> Result<()> {
    let modules = root.join("node_modules");
    safe_directory(&modules)?;
    if !modules.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&modules)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let names = if name.starts_with('@') {
            ensure!(
                !entry.file_type()?.is_symlink(),
                "global package scope must not be a symlink"
            );
            fs::read_dir(entry.path())?
                .map(|e| Ok(format!("{name}/{}", e?.file_name().to_string_lossy())))
                .collect::<std::io::Result<Vec<_>>>()?
        } else {
            vec![name]
        };
        for name in names {
            ensure!(
                lock.is_some_and(|l| l.packages.contains_key(&crate::lockfile::child("", &name))),
                "refusing to remove unmanaged global package {name}; use a dedicated --prefix"
            );
        }
    }
    Ok(())
}

fn sync_bins(prefix: &Path, root: &Path) -> Result<()> {
    let bin = if cfg!(windows) {
        prefix.to_owned()
    } else {
        prefix.join("bin")
    };
    safe_directory(&bin)?;
    fs::create_dir_all(&bin)?;
    let marker = root.join(".tnpm-global-bins.json");
    ensure!(
        !fs::symlink_metadata(&marker).is_ok_and(|m| m.file_type().is_symlink()),
        "global ownership file must not be a symlink"
    );
    let previous = if marker.exists() {
        Value::parse(&fs::read_to_string(&marker)?)?
    } else {
        Value::object()
    };
    let data = manifest::read(root)?.context("global manifest missing")?;
    let mut desired = Value::object();
    for dep in crate::lockfile::dependencies(&data, true)
        .into_iter()
        .filter(|d| !d.peer)
    {
        let package = root.join("node_modules").join(&dep.name);
        let Some(data) = manifest::read(&package)? else {
            continue;
        };
        for (name, target) in scripts::bins(&data) {
            ensure!(
                !name.is_empty()
                    && !name.contains(['/', '\\', '\0', '\n', '\r'])
                    && name != "."
                    && name != "..",
                "unsafe global binary name"
            );
            let source = fs::canonicalize(package.join(target))?;
            ensure!(
                source.starts_with(fs::canonicalize(&package)?),
                "global binary escapes package"
            );
            #[cfg(windows)]
            let name = format!("{name}.cmd");
            if desired.get(&name).is_some() {
                bail!("conflicting global binary: {name}");
            }
            desired.set(&name, Value::from(source.to_string_lossy().as_ref()));
        }
    }
    fn owned(path: &Path, old: Option<&Value>) -> bool {
        let Some(source) = old.and_then(Value::string) else {
            return false;
        };
        #[cfg(unix)]
        {
            fs::read_link(path).is_ok_and(|target| target == Path::new(source))
        }
        #[cfg(windows)]
        {
            fs::read_to_string(path).is_ok_and(|text| text == shim(source))
        }
    }
    // Check every collision before changing any executable.
    for (name, _) in desired.members() {
        let path = bin.join(name);
        if fs::symlink_metadata(&path).is_ok() && !owned(&path, previous.get(name)) {
            bail!(
                "refusing to overwrite unowned global binary: {}",
                path.display()
            );
        }
    }
    for (name, value) in previous.members() {
        ensure!(
            !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..",
            "unsafe global ownership record"
        );
        let path = bin.join(name);
        if owned(&path, Some(value)) {
            fs::remove_file(&path)?;
        }
    }
    for (name, source) in desired.members() {
        let source = source.string().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(source, bin.join(name))?;
        #[cfg(windows)]
        fs::write(bin.join(name), shim(source))?;
    }
    write_json(&marker, &desired)?;
    Ok(())
}

#[cfg(windows)]
fn shim(source: &str) -> String {
    format!("@echo off\r\nnode \"{source}\" %*\r\n")
}
