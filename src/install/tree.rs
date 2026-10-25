//! Installed-tree scanning, safe mutation, binary links, and lifecycle execution.
use super::{CONCURRENCY, context::Context, flag, store, summary};
use crate::{
    fs::{unique, write_json},
    json::Value,
    lockfile::{self, Lockfile},
    manifest, scripts,
};
use anyhow::{Context as _, Result, bail};
use futures_util::{StreamExt, stream};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
};

use store::{
    cache_extracted_package, cached_package, clone_tree, package_cache_path, safe_relative,
    tarball, unpack,
};

fn retain_reachable(
    lock: &Lockfile,
    manifest: &Value,
    selected: &mut BTreeSet<String>,
) -> Result<()> {
    let used = lock.reachable_selected(manifest, selected)?;
    selected.retain(|p| used.contains(p));
    Ok(())
}

fn platform(data: &Value) -> bool {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let cpu = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    fn accepts(data: Option<&Value>, current: &str) -> bool {
        let Some(Value::Array(values)) = data else {
            return true;
        };
        let values: Vec<_> = values.iter().filter_map(Value::string).collect();
        !values.iter().any(|v| *v == format!("!{current}"))
            && (!values.iter().any(|v| !v.starts_with('!'))
                || values.contains(&current)
                || values.contains(&"any"))
    }
    accepts(data.get("os"), os) && accepts(data.get("cpu"), cpu)
}

fn ensure_safe_parents(root: &Path, relative: &str) -> Result<()> {
    let mut path = root.to_owned();
    for component in Path::new(relative).components() {
        path.push(component);
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.file_type().is_symlink()
        {
            bail!(
                "refusing to modify symlinked installation path: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn hidden_lock(root: &Path) -> Option<(BTreeMap<String, Value>, std::time::SystemTime)> {
    let path = root.join("node_modules/.package-lock.json");
    let modified = fs::metadata(&path).ok()?.modified().ok()?;
    let value = Value::parse(&fs::read_to_string(path).ok()?).ok()?;
    if value.get("lockfileVersion") != Some(&Value::Number("3".into())) {
        return None;
    }
    let entries = value.get("packages")?;
    if !matches!(entries, Value::Object(_)) {
        return None;
    }
    let mut packages = BTreeMap::new();
    for (path, entry) in entries.members() {
        if !lockfile::valid_location(path)
            || !matches!(entry, Value::Object(_))
            || flag(entry, "link")
        {
            return None;
        }
        let directory = root.join(path);
        if fs::symlink_metadata(&directory)
            .ok()?
            .file_type()
            .is_symlink()
            || !directory.join("package.json").is_file()
            || fs::metadata(directory).ok()?.modified().ok()? > modified
        {
            return None;
        }
        packages.insert(path.clone(), entry.clone());
    }
    Some((packages, modified))
}

fn scan(root: &Path) -> Result<Lockfile> {
    let cached = hidden_lock(root);
    let result = scan_using(root, cached.as_ref().map(|(p, _)| p))?;
    // Any extra directory invalidates the *whole* hidden lock, not just that entry.
    if let Some((cached, _)) = cached
        && cached.keys().ne(result.packages.keys())
    {
        return scan_using(root, None);
    }
    Ok(result)
}

pub(super) fn snapshot(root: &Path) -> Result<Lockfile> {
    ensure_safe_parents(root, "node_modules")?;
    scan(root)
}

fn scan_using(root: &Path, cached: Option<&BTreeMap<String, Value>>) -> Result<Lockfile> {
    let mut lock = Lockfile::default();
    let mut queue = VecDeque::from([String::new()]);
    while let Some(parent) = queue.pop_front() {
        let modules = root.join(&parent).join("node_modules");
        let entries = match fs::read_dir(&modules) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let mut names = Vec::new();
            if name.starts_with('@') && !entry.file_type()?.is_symlink() && entry.path().is_dir() {
                for child in fs::read_dir(entry.path())? {
                    let child = child?;
                    names.push(format!("{name}/{}", child.file_name().to_string_lossy()));
                }
            } else {
                names.push(name);
            }
            for name in names {
                if !lockfile::valid_name(&name) {
                    continue;
                }
                let location = lockfile::child(&parent, &name);
                let path = root.join(&location);
                let symlink = fs::symlink_metadata(&path)?.file_type().is_symlink();
                let data = if let Some(data) = cached.and_then(|c| c.get(&location)) {
                    data.clone()
                } else {
                    manifest::read(&path)?.unwrap_or_else(Value::object)
                };
                lock.packages.insert(location.clone(), data);
                if !symlink && location.matches("node_modules/").count() < 128 {
                    queue.push_back(location);
                }
            }
        }
    }
    Ok(lock)
}

fn link_bins(root: &Path, paths: &BTreeSet<String>) -> Result<()> {
    let mut bin_dirs = BTreeSet::from([root.join("node_modules/.bin")]);
    for path in paths {
        let package = root.join(path);
        let modules = if lockfile::name_at(path).starts_with('@') {
            package.parent().unwrap().parent().unwrap()
        } else {
            package.parent().unwrap()
        };
        bin_dirs.insert(modules.join(".bin"));
    }
    for dir in &bin_dirs {
        if let Ok(relative) = dir.strip_prefix(root) {
            ensure_safe_parents(root, &relative.to_string_lossy())?;
        }
        // .bin is generated exclusively by package installation.
        remove_path(dir)?;
        fs::create_dir_all(dir)?;
    }
    for path in paths {
        let package = root.join(path);
        let Some(data) = manifest::read(&package)? else {
            continue;
        };
        let modules = if lockfile::name_at(path).starts_with('@') {
            package.parent().unwrap().parent().unwrap()
        } else {
            package.parent().unwrap()
        };
        let dir = modules.join(".bin");
        fs::create_dir_all(&dir)?;
        for (name, target) in scripts::bins(&data) {
            if name.is_empty()
                || name.contains(['/', '\\'])
                || name == "."
                || name == ".."
                || !safe_relative(Path::new(&target))
            {
                bail!("unsafe bin entry: {name} -> {target}");
            }
            let source = package.join(&target);
            if !source.is_file() {
                continue;
            }
            let canonical = fs::canonicalize(&source)?;
            if !canonical.starts_with(fs::canonicalize(&package)?) {
                bail!("bin target escapes package");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::{PermissionsExt, symlink};
                fs::set_permissions(
                    &source,
                    fs::Permissions::from_mode(fs::metadata(&source)?.permissions().mode() | 0o111),
                )?;
                let destination = dir.join(&name);
                if fs::symlink_metadata(&destination).is_ok() {
                    continue;
                }
                let relative = PathBuf::from("..")
                    .join(lockfile::name_at(path))
                    .join(target);
                symlink(relative, destination)?;
            }
            #[cfg(windows)]
            {
                fs::write(
                    dir.join(format!("{name}.cmd")),
                    format!("@echo off\r\nnode \"{}\" %*\r\n", source.display()),
                )?;
            }
        }
    }
    Ok(())
}

pub(super) async fn materialize(
    ctx: &Context<'_>,
    root: &Path,
    manifest: &Value,
    lock: &Lockfile,
    clean: bool,
) -> Result<()> {
    ctx.warn_peers(lock);
    ensure_safe_parents(root, "node_modules")?;
    // Validate all paths before mutating the tree.
    for path in lock.packages.keys().filter(|p| !p.is_empty()) {
        ensure_safe_parents(root, path)?;
    }
    let scripts_enabled = !ctx.options.ignore_scripts && !ctx.config.enabled("ignore-scripts");
    if clean {
        ctx.progress
            .event("place", "removing existing node_modules for clean install");
        remove_path(&root.join("node_modules"))?;
    }
    if scripts_enabled {
        ctx.lifecycle(root, manifest, "preinstall")?;
    }
    fs::create_dir_all(root.join("node_modules"))?;
    let mut selected = BTreeSet::new();
    for (path, entry) in &lock.packages {
        if path.is_empty() || ctx.omit(entry) {
            continue;
        }
        if !platform(entry) {
            if flag(entry, "optional") {
                continue;
            }
            bail!("EBADPLATFORM: unsupported OS/CPU for {path}");
        }
        selected.insert(path.clone());
    }
    retain_reachable(lock, manifest, &mut selected)?;
    ctx.progress.event(
        "prepare",
        &format!("scanning {}", root.join("node_modules").display()),
    );
    let installed = scan(root)?;
    let mut changed = BTreeSet::new();
    let mut jobs: BTreeSet<_> = selected
        .iter()
        .filter(|path| {
            let entry = &lock.packages[*path];
            !installed.packages.get(*path).is_some_and(|data| {
                data.str("version") == entry.str("version")
                    && root.join(path).join("package.json").is_file()
                    && ["resolved", "integrity"]
                        .iter()
                        .all(|key| data.get(key).is_none() || data.get(key) == entry.get(key))
            })
        })
        .cloned()
        .collect();
    for path in &selected {
        let mut parent = lockfile::parent(path);
        while !parent.is_empty() {
            if jobs.contains(parent) {
                jobs.insert(path.clone());
                break;
            }
            parent = lockfile::parent(parent);
        }
    }
    let jobs: Vec<_> = jobs.into_iter().collect();
    for path in &jobs {
        ctx.progress.start("prepare", path);
    }
    if jobs.is_empty() {
        ctx.progress
            .event("prepare", "all installed packages are reusable");
    }
    // Clone cached packages or download/decompress concurrently, then commit parent-first.
    let stage = root.join(format!(".tnpm-stage-{}-{}", std::process::id(), unique()));
    fs::create_dir(&stage)?;
    let result = async {
        let results: Vec<_> = stream::iter(jobs.iter().enumerate().map(|(i, path)| {
            let destination = stage.join(i.to_string());
            async move {
                let entry = &lock.packages[path];
                let result = async {
                    let package_cache = package_cache_path(ctx, entry);
                    if let Some(cached) = package_cache.as_deref().and_then(cached_package) {
                        ctx.progress
                            .event("prepare", &format!("clone package cache {path}"));
                        let dest = destination.clone();
                        let cache_hit =
                            tokio::task::spawn_blocking(move || clone_tree(&cached, &dest)).await?;
                        if cache_hit.is_ok() {
                            return Ok::<_, anyhow::Error>(());
                        }
                        // A stale/partially damaged extracted cache is only an
                        // optimization failure; fall back to the verified tarball.
                        remove_path(&destination)?;
                    }
                    let bytes = tarball(ctx, entry).await?;
                    ctx.progress.event("prepare", &format!("extract {path}"));
                    let dest = destination.clone();
                    let cache = package_cache.clone();
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        fs::create_dir_all(&dest)?;
                        unpack(&bytes, &dest).context("could not unpack tarball")?;
                        if let Some(cache) = cache
                            && let Err(error) = cache_extracted_package(&dest, &cache)
                        {
                            crate::progress::message(format_args!(
                                "tnpm: could not cache extracted package: {error}"
                            ));
                        }
                        Ok(())
                    })
                    .await
                    .context("package extraction task failed")??;
                    Ok::<_, anyhow::Error>(())
                }
                .await
                .with_context(|| format!("could not prepare {path}"));
                ctx.progress.finish(
                    "prepare",
                    path,
                    if result.is_ok() {
                        "prepared"
                    } else if flag(entry, "optional") {
                        "skipped"
                    } else {
                        "failed"
                    },
                );
                (path.clone(), destination, result)
            }
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
        let mut prepared = BTreeMap::new();
        for (path, destination, result) in results {
            match result {
                Ok(()) => {
                    prepared.insert(path, destination);
                }
                Err(e) if flag(&lock.packages[&path], "optional") => {
                    crate::progress::message(format_args!("tnpm: skipped optional {path}: {e:#}"));
                    selected.remove(&path);
                }
                Err(e) => return Err(e),
            }
        }
        retain_reachable(lock, manifest, &mut selected)?;
        prepared.retain(|path, _| selected.contains(path));
        for path in prepared.keys() {
            ctx.progress.start("place", path);
        }
        if prepared.is_empty() {
            ctx.progress.event("place", "no packages need placement");
        }
        // BTree order guarantees parent locations precede their nested packages.
        for (path, source) in prepared {
            let destination = root.join(&path);
            ensure_safe_parents(root, &path)?;
            remove_path(&destination)?;
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::rename(source, destination)?;
            ctx.progress.finish(
                "place",
                &path,
                &format!(
                    "installed {} at",
                    lock.packages[&path].str("version").unwrap_or("")
                ),
            );
            changed.insert(path);
        }
        for path in installed.packages.keys().rev() {
            if !selected.contains(path) {
                ctx.progress.start("remove", path);
                ensure_safe_parents(root, path)?;
                remove_path(&root.join(path))?;
                ctx.progress.finish("remove", path, "removed");
            }
        }
        let tree_unchanged = changed.is_empty() && installed.packages.keys().eq(selected.iter());
        let bins_exist = selected.iter().all(|path| {
            let data = &lock.packages[path];
            let modules = root.join(lockfile::parent(path)).join("node_modules/.bin");
            scripts::bins(data)
                .iter()
                .all(|(name, _)| modules.join(name).is_file())
        });
        if !tree_unchanged || !bins_exist {
            ctx.progress.event("place", "linking package binaries");
            link_bins(root, &selected)?;
        }
        if scripts_enabled {
            // Nested dependencies first; all binaries already exist before any lifecycle hook.
            let mut order: Vec<_> = changed.iter().collect();
            order.sort_by_key(|p| std::cmp::Reverse(p.matches("node_modules/").count()));
            for path in order {
                if !selected.contains(path) {
                    continue;
                }
                let directory = root.join(path);
                if let Some(data) = manifest::read(&directory)? {
                    for event in ["preinstall", "install", "postinstall"] {
                        if let Err(error) = ctx.lifecycle(&directory, &data, event) {
                            if !flag(&lock.packages[path], "optional") {
                                return Err(error);
                            }
                            crate::progress::message(format_args!(
                                "tnpm: skipped optional {path}: {error:#}"
                            ));
                            selected.remove(path);
                            let previous = selected.clone();
                            retain_reachable(lock, manifest, &mut selected)?;
                            remove_path(&directory)?;
                            for removed in previous.difference(&selected) {
                                remove_path(&root.join(removed))?;
                            }
                            link_bins(root, &selected)?;
                            break;
                        }
                    }
                }
            }
            for event in [
                "install",
                "postinstall",
                "prepublish",
                "preprepare",
                "prepare",
                "postprepare",
            ] {
                ctx.lifecycle(root, manifest, event)?;
            }
        }
        let mut hidden = lock.clone();
        hidden.packages.retain(|path, _| selected.contains(path));
        write_json(
            &root.join("node_modules/.package-lock.json"),
            &hidden.value(manifest, true),
        )?;
        Ok(())
    }
    .await;
    let _ = fs::remove_dir_all(&stage);
    result?;
    if ctx.options.command != "update" {
        summary(ctx.options, "installed", changed.len());
    }
    Ok(())
}

pub(super) fn prune(ctx: &Context<'_>, root: &Path) -> Result<()> {
    if ctx.options.lock_only {
        bail!("prune --package-lock-only is not supported");
    }
    let manifest = manifest::read(root)?.context("package.json not found")?;
    let old = Lockfile::read(root)?;
    let mut installed = scan(root)?;
    let used = installed.reachable_with(&manifest, false, ctx.peer_policy())?;
    installed.classify_with(&manifest, ctx.peer_policy());
    let mut retained = BTreeSet::new();
    let mut removed = 0;
    for (path, entry) in installed.packages.iter().rev() {
        let filtered = !ctx.options.arguments.is_empty()
            && !ctx
                .options
                .arguments
                .iter()
                .any(|n| n == lockfile::name_at(path));
        if (!used.contains(path) || ctx.omit(entry)) && !filtered {
            ensure_safe_parents(root, path)?;
            remove_path(&root.join(path))?;
            removed += 1;
        } else {
            retained.insert(path.clone());
        }
    }
    // Parent removal may also remove previously retained nested extras.
    retained.retain(|path| root.join(path).join("package.json").is_file());
    if !ctx.options.lock_only {
        link_bins(root, &retained)?;
    }
    if let Some(mut lock) = old {
        let used = lock.reachable_with(&manifest, false, ctx.peer_policy())?;
        lock.packages
            .retain(|p, _| p.is_empty() || used.contains(p));
        lock.classify_with(&manifest, ctx.peer_policy());
        if !ctx.options.no_save {
            lock.write(root, &manifest)?;
        }
    }
    remove_path(&root.join("node_modules/.package-lock.json"))?;
    summary(ctx.options, "removed", removed);
    Ok(())
}
