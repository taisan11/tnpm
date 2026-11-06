//! Local snapshots, workspace links, and Git archives pinned to immutable commits.
use super::{context::Context, store};
use crate::{json::Value, lockfile, manifest, semver, workspaces};
use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha512};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub(super) fn is_source(spec: &str) -> bool {
    spec.starts_with("file:") || spec.starts_with("workspace:") || is_git(spec)
}

pub(super) fn is_git(spec: &str) -> bool {
    spec.starts_with("git+")
        || spec.starts_with("git://")
        || spec.starts_with("git@")
        || spec.starts_with("github:")
        || spec.starts_with("gitlab:")
        || spec.starts_with("bitbucket:")
        || ((spec.starts_with("https://") || spec.starts_with("ssh://"))
            && spec.split('#').next().unwrap_or(spec).ends_with(".git"))
        || github_shorthand(spec)
}

fn github_shorthand(spec: &str) -> bool {
    let path = spec.split('#').next().unwrap_or(spec);
    let Some((owner, repo)) = path.split_once('/') else {
        return false;
    };
    [owner, repo].iter().all(|p| {
        !p.is_empty()
            && *p != "."
            && *p != ".."
            && p.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    })
}

pub(super) fn relative(root: &Path, target: &Path) -> String {
    if matches!((root.components().next(), target.components().next()), (Some(Component::Prefix(a)), Some(Component::Prefix(b))) if a != b)
    {
        return target.to_string_lossy().replace('\\', "/");
    }
    let root = root.components().collect::<Vec<_>>();
    let target = target.components().collect::<Vec<_>>();
    let shared = root.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let path = std::iter::repeat_n("..".to_owned(), root.len() - shared)
        .chain(
            target[shared..]
                .iter()
                .map(|c| c.as_os_str().to_string_lossy().into_owned()),
        )
        .collect::<Vec<_>>()
        .join("/");
    if path.is_empty() {
        ".".to_owned()
    } else {
        path
    }
}

pub(super) fn effective(
    root: &Path,
    manifest: &Value,
) -> Result<(Value, Vec<workspaces::Workspace>)> {
    let items = workspaces::discover(root, manifest)?;
    let mut effective = manifest.clone();
    for item in &items {
        if effective.get("dependencies").is_none() {
            effective.set("dependencies", Value::object());
        }
        // Every workspace is linked, including workspaces not declared as dependencies.
        let existing = lockfile::dependencies(&effective, true)
            .into_iter()
            .find(|d| d.name == item.name && !d.peer);
        if let Some(dep) = existing {
            let version = item.manifest.str("version").unwrap_or("0.0.0");
            if !dep.spec.starts_with("workspace:")
                && semver::satisfies(version, &dep.spec) != Some(true)
            {
                bail!(
                    "workspace {} conflicts with root dependency {}",
                    item.name,
                    dep.spec
                );
            }
        } else {
            effective
                .get_mut("dependencies")
                .unwrap()
                .set(&item.name, Value::from("workspace:*"));
        }
    }
    Ok((effective, items))
}

fn rewrite_relative(data: &mut Value, directory: &Path, root: &Path) -> Result<()> {
    for field in lockfile::DEP_FIELDS {
        let Some(deps) = data.get_mut(field) else {
            continue;
        };
        for (name, value) in deps.members().to_vec() {
            if let Some(path) = value.string().and_then(|v| v.strip_prefix("file:")) {
                let target = fs::canonicalize(directory.join(path))
                    .with_context(|| format!("local dependency not found: {name} -> {path}"))?;
                deps.set(
                    &name,
                    Value::from(format!("file:{}", relative(root, &target)).as_str()),
                );
            }
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))
}

fn source_data(
    mut data: Value,
    spec: &str,
    resolved: &str,
    integrity: Option<String>,
) -> Result<Value> {
    let name = data.str("name").context("source package requires a name")?;
    ensure!(
        lockfile::valid_name(name),
        "invalid source package name: {name}"
    );
    if data.str("version").is_none() {
        data.set("version", Value::from("0.0.0"));
    }
    ensure!(
        semver::Version::parse(data.str("version").unwrap()).is_some(),
        "invalid source package version"
    );
    data.set("tnpmSpec", Value::from(spec));
    let mut dist = Value::object();
    dist.set("tarball", Value::from(resolved));
    if let Some(integrity) = integrity {
        dist.set("integrity", Value::from(integrity.as_str()));
    }
    data.set("dist", dist);
    Ok(data)
}

pub(super) async fn metadata(ctx: &Context<'_>, name: &str, spec: &str) -> Result<Option<Value>> {
    let root = ctx.source_root.borrow().clone();
    if let Some(item) = ctx
        .workspaces
        .borrow()
        .iter()
        .find(|w| w.name == name)
        .cloned()
    {
        let version = item.manifest.str("version").unwrap_or("0.0.0");
        let range = spec.strip_prefix("workspace:").unwrap_or(spec);
        if matches!(range, "*" | "^" | "~") || semver::satisfies(version, range) == Some(true) {
            let directory = fs::canonicalize(root.join(&item.relative))?;
            let mut data = item.manifest;
            rewrite_relative(&mut data, &directory, &root)?;
            data.set("link", Value::Bool(true));
            data.set("tnpmWorkspace", Value::from(item.relative.as_str()));
            return source_data(data, spec, &format!("file:{}", item.relative), None).map(Some);
        }
        if spec.starts_with("workspace:") {
            bail!("workspace version does not satisfy {name}@{spec}");
        }
    }
    if spec.starts_with("workspace:") {
        bail!("workspace not found: {name}");
    }
    if let Some(path) = spec.strip_prefix("file:") {
        let path = fs::canonicalize(root.join(path))
            .with_context(|| format!("local dependency not found: {spec}"))?;
        if path.is_dir() {
            let mut data =
                manifest::read(&path)?.context("local dependency has no package.json")?;
            rewrite_relative(&mut data, &path, &root)?;
            return source_data(
                data,
                spec,
                &format!("file:{}", relative(&root, &path)),
                None,
            )
            .map(Some);
        }
        let bytes = fs::read(&path)?;
        let temp = tempfile::tempdir()?;
        store::unpack(&bytes, temp.path())?;
        let data = manifest::read(temp.path())?.context("local archive has no package.json")?;
        return source_data(
            data,
            spec,
            &format!("file:{}", relative(&root, &path)),
            Some(digest(&bytes)),
        )
        .map(Some);
    }
    if is_git(spec) {
        let spec_owned = spec.to_owned();
        let offline = ctx.offline();
        let no_cache = ctx.options.no_cache;
        let bytes_and_resolved =
            tokio::task::spawn_blocking(move || git_archive(&spec_owned, offline, no_cache))
                .await??;
        let (bytes, resolved) = bytes_and_resolved;
        let temp = tempfile::tempdir()?;
        store::unpack(&bytes, temp.path())?;
        let mut data =
            manifest::read(temp.path())?.context("Git dependency has no package.json")?;
        if lockfile::dependencies(&data, false)
            .iter()
            .any(|d| d.spec.starts_with("file:"))
        {
            bail!("Git dependencies with repository-relative file dependencies are not supported");
        }
        if data.get("scripts").and_then(|s| s.str("prepare")).is_some() {
            let mut deps = data
                .get("devDependencies")
                .cloned()
                .unwrap_or_else(Value::object);
            for (key, value) in data
                .get("dependencies")
                .map(Value::members)
                .unwrap_or_default()
            {
                deps.set(key, value.clone());
            }
            data.set("dependencies", deps);
        }
        return source_data(data, spec, &resolved, Some(digest(&bytes))).map(Some);
    }
    Ok(None)
}

pub(super) fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fn copy(source: &Path, destination: &Path, depth: usize) -> Result<()> {
        ensure!(depth <= 128, "local dependency nesting exceeds 128 levels");
        let metadata = fs::symlink_metadata(source)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "local dependency contains symlink: {}",
            source.display()
        );
        if metadata.is_dir() {
            fs::create_dir(destination)?;
            for entry in fs::read_dir(source)? {
                let entry = entry?;
                let name = entry.file_name();
                if name == "node_modules"
                    || name == ".git"
                    || name.to_string_lossy().starts_with(".tnpm-stage-")
                {
                    continue;
                }
                copy(&entry.path(), &destination.join(name), depth + 1)?;
            }
        } else if metadata.is_file() {
            fs::copy(source, destination)?;
        } else {
            bail!(
                "local dependency contains special file: {}",
                source.display()
            );
        }
        Ok(())
    }
    copy(source, destination, 0)
}

pub(super) async fn prepare(ctx: &Context<'_>, entry: &Value, destination: &Path) -> Result<bool> {
    let Some(resolved) = entry.str("resolved") else {
        return Ok(false);
    };
    if let Some(path) = resolved.strip_prefix("file:") {
        let source = fs::canonicalize(ctx.source_root.borrow().join(path))?;
        if entry.get("link") == Some(&Value::Bool(true)) {
            let relative = entry
                .str("tnpmWorkspace")
                .context("workspace link missing source")?;
            ensure!(
                store::safe_relative(Path::new(relative)),
                "unsafe workspace link"
            );
            let root = fs::canonicalize(&*ctx.source_root.borrow())?;
            ensure!(
                source == root.join(relative) && source.starts_with(&root),
                "workspace source escapes project"
            );
            #[cfg(unix)]
            std::os::unix::fs::symlink(&source, destination)?;
            #[cfg(windows)]
            std::os::windows::fs::symlink_dir(&source, destination).context(
                "workspace links require Developer Mode or symlink privileges on Windows",
            )?;
        } else if source.is_dir() {
            ensure!(
                !destination.starts_with(&source),
                "local dependency cannot contain installation staging directory"
            );
            copy_directory(&source, destination)?;
        } else {
            let bytes = fs::read(source)?;
            store::verify(&bytes, entry.str("integrity"))?;
            fs::create_dir(destination)?;
            store::unpack(&bytes, destination)?;
        }
        return Ok(true);
    }
    if is_git(resolved) {
        let resolved = resolved.to_owned();
        let offline = ctx.offline();
        let no_cache = ctx.options.no_cache;
        let (bytes, _) =
            tokio::task::spawn_blocking(move || git_archive(&resolved, offline, no_cache))
                .await??;
        store::verify(&bytes, entry.str("integrity"))?;
        fs::create_dir(destination)?;
        store::unpack(&bytes, destination)?;
        return Ok(true);
    }
    Ok(false)
}

fn git_spec(spec: &str) -> Result<(String, String)> {
    let (url, reference) = spec.rsplit_once('#').unwrap_or((spec, "HEAD"));
    let url = if let Some(path) = url.strip_prefix("github:") {
        format!("https://github.com/{path}.git")
    } else if let Some(path) = url.strip_prefix("gitlab:") {
        format!("https://gitlab.com/{path}.git")
    } else if let Some(path) = url.strip_prefix("bitbucket:") {
        format!("https://bitbucket.org/{path}.git")
    } else if github_shorthand(url) {
        format!("https://github.com/{}.git", url.trim_end_matches(".git"))
    } else {
        url.strip_prefix("git+").unwrap_or(url).to_owned()
    };
    ensure!(
        url.starts_with("https://")
            || url.starts_with("ssh://")
            || url.starts_with("git://")
            || url.starts_with("file://")
            || url.starts_with("git@"),
        "unsupported Git transport"
    );
    ensure!(
        !reference.is_empty()
            && !reference.starts_with('-')
            && !reference.contains(['\0', '\n', '\r']),
        "unsupported Git reference"
    );
    if let Some(range) = reference.strip_prefix("semver:") {
        ensure!(
            semver::satisfies("0.0.0", range).is_some(),
            "invalid Git semver range"
        );
    }
    if let Ok(url) = reqwest::Url::parse(&url) {
        ensure!(
            url.password().is_none() && (url.scheme() == "ssh" || url.username().is_empty()),
            "Git URL credentials must use a credential helper"
        );
    }
    Ok((url, reference.to_owned()))
}

fn git(args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "core.hooksPath=",
            "-c",
            "credential.interactive=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .context("Git dependencies require git on PATH")?;
    ensure!(
        output.status.success(),
        "Git dependency operation failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

fn git_archive(spec: &str, offline: bool, no_cache: bool) -> Result<(Vec<u8>, String)> {
    let (url, mut reference) = git_spec(spec)?;
    let pinned =
        matches!(reference.len(), 40 | 64) && reference.bytes().all(|b| b.is_ascii_hexdigit());
    let cache = dirs::home_dir()
        .context("home directory not found")?
        .join(".tnpm/cache/git-v1");
    if !no_cache {
        for directory in cache.ancestors().take(3) {
            if let Ok(metadata) = fs::symlink_metadata(directory) {
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "Git cache directory must not be a symlink: {}",
                    directory.display()
                );
            }
        }
    }
    let key = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{url}#{reference}").as_bytes())
    );
    let cached = cache.join(format!("{key}.tar"));
    if !no_cache
        && pinned
        && fs::symlink_metadata(&cached).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
    {
        return Ok((fs::read(cached)?, format!("git+{url}#{reference}")));
    }
    ensure!(!offline, "ENOTCACHED: Git dependency {spec}");
    let temp = tempfile::tempdir()?;
    let repo = temp.path().join("repository");
    git(&[
        "clone".as_ref(),
        "--quiet".as_ref(),
        "--no-checkout".as_ref(),
        "--template=".as_ref(),
        "--".as_ref(),
        url.as_ref(),
        repo.as_os_str(),
    ])?;
    if let Some(range) = reference.strip_prefix("semver:") {
        let tags = git(&[
            "-C".as_ref(),
            repo.as_os_str(),
            "for-each-ref".as_ref(),
            "--format=%(refname:short)".as_ref(),
            "refs/tags".as_ref(),
        ])?;
        reference = String::from_utf8(tags)?
            .lines()
            .filter_map(|tag| {
                let version = semver::Version::parse(tag)?;
                (semver::satisfies(tag, range) == Some(true)).then(|| (version, tag.to_owned()))
            })
            .max_by(|(a, _), (b, _)| a.compare(b))
            .map(|(_, tag)| format!("refs/tags/{tag}"))
            .context("no Git tag satisfies semver range")?;
    }
    let commit = git(&[
        "-C".as_ref(),
        repo.as_os_str(),
        "rev-parse".as_ref(),
        "--verify".as_ref(),
        "--end-of-options".as_ref(),
        format!("{reference}^{{commit}}").as_ref(),
    ])
    .or_else(|_| {
        git(&[
            "-C".as_ref(),
            repo.as_os_str(),
            "rev-parse".as_ref(),
            "--verify".as_ref(),
            "--end-of-options".as_ref(),
            format!("origin/{reference}^{{commit}}").as_ref(),
        ])
    })?;
    let commit = String::from_utf8(commit)?.trim().to_owned();
    ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid Git commit"
    );
    let bytes = git(&[
        "-C".as_ref(),
        repo.as_os_str(),
        "archive".as_ref(),
        "--format=tar".as_ref(),
        "--prefix=package/".as_ref(),
        commit.as_ref(),
    ])?;
    if no_cache {
        return Ok((bytes, format!("git+{url}#{commit}")));
    }
    fs::create_dir_all(&cache)?;
    ensure!(
        !fs::symlink_metadata(&cache)?.file_type().is_symlink(),
        "Git cache must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o700))?;
    }
    let key = format!(
        "{:x}",
        sha2::Sha256::digest(format!("{url}#{commit}").as_bytes())
    );
    let path = cache.join(format!("{key}.tar"));
    let mut temporary = tempfile::NamedTempFile::new_in(&cache)?;
    use std::io::Write;
    temporary.write_all(&bytes)?;
    temporary.persist(&path).map_err(|e| e.error)?;
    Ok((bytes, format!("git+{url}#{commit}")))
}

pub(super) fn workspace_target(ctx: &Context<'_>, entry: &Value) -> Result<PathBuf> {
    let relative = entry
        .str("tnpmWorkspace")
        .context("workspace link missing source")?;
    ensure!(
        store::safe_relative(Path::new(relative)),
        "unsafe workspace source"
    );
    let root = fs::canonicalize(&*ctx.source_root.borrow())?;
    let target = fs::canonicalize(root.join(relative))?;
    ensure!(
        target == root.join(relative) && target.starts_with(&root),
        "workspace source escapes project"
    );
    // Never accept a target from a lockfile alone: it must be a declared workspace.
    ensure!(
        ctx.workspaces
            .borrow()
            .iter()
            .any(|w| w.relative == relative && Some(w.name.as_str()) == entry.str("name")),
        "undeclared workspace link"
    );
    ensure!(
        !Path::new(relative)
            .components()
            .any(|c| matches!(c, Component::ParentDir)),
        "unsafe workspace source"
    );
    Ok(target)
}
