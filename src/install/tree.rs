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

fn build_state(root: &Path) -> BTreeMap<String, Value> {
    fs::read_to_string(root.join("node_modules/.tnpm-lock.json"))
        .ok()
        .and_then(|text| Value::parse(&text).ok())
        .and_then(|value| value.get("tnpmBuilds").cloned())
        .map(|value| value.members().iter().cloned().collect())
        .unwrap_or_default()
}

fn build_fingerprint(entry: &Value, data: &Value, directory: &Path) -> Value {
    let mut value = lockfile::copy_fields(entry, &["name", "version", "resolved", "integrity"]);
    value.set(
        "scripts",
        data.get("scripts").cloned().unwrap_or(Value::Null),
    );
    value.set(
        "native",
        Value::Bool(scripts::implicit_native(directory, data)),
    );
    value
}

fn retain_reachable(
    lock: &Lockfile,
    manifest: &Value,
    selected: &mut BTreeSet<String>,
) -> Result<()> {
    let used = lock.reachable_selected(manifest, selected)?;
    selected.retain(|p| used.contains(p));
    Ok(())
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

fn ensure_install_path(
    ctx: &Context<'_>,
    root: &Path,
    relative: &str,
    lock: &Lockfile,
) -> Result<()> {
    let mut current = root.to_owned();
    let mut logical = String::new();
    for component in Path::new(relative).components() {
        current.push(component);
        if !logical.is_empty() {
            logical.push('/');
        }
        logical.push_str(&component.as_os_str().to_string_lossy());
        if let Ok(meta) = fs::symlink_metadata(&current)
            && meta.file_type().is_symlink()
        {
            let entry = lock
                .packages
                .get(&logical)
                .filter(|e| flag(e, "link"))
                .with_context(|| {
                    format!(
                        "refusing to modify symlinked installation path: {}",
                        current.display()
                    )
                })?;
            let target = super::sources::workspace_target(ctx, entry)?;
            if fs::canonicalize(&current)? != target {
                bail!(
                    "workspace link points to an unexpected target: {}",
                    current.display()
                );
            }
            current = target;
        }
    }
    Ok(())
}

pub(super) fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => fs::remove_dir_all(path)?,
        Ok(_) => fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn hidden_lock(root: &Path) -> Option<(BTreeMap<String, Value>, std::time::SystemTime)> {
    let path = root.join("node_modules/.tnpm-lock.json");
    let modified = fs::metadata(&path).ok()?.modified().ok()?;
    let value = Value::parse(&fs::read_to_string(path).ok()?).ok()?;
    if value.get("tnpmLockfileVersion") != Some(&Value::Number("1".into())) {
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

fn scan(ctx: &Context<'_>, root: &Path) -> Result<Lockfile> {
    let cached = hidden_lock(root);
    let result = scan_using(ctx, root, cached.as_ref().map(|(p, _)| p))?;
    // Any extra directory invalidates the *whole* hidden lock, not just that entry.
    if let Some((cached, _)) = cached
        && cached.keys().ne(result.packages.keys())
    {
        return scan_using(ctx, root, None);
    }
    Ok(result)
}

pub(super) fn snapshot(ctx: &Context<'_>, root: &Path) -> Result<Lockfile> {
    ensure_safe_parents(root, "node_modules")?;
    if super::isolated::active(root) {
        let mut lock = Lockfile::read(root)?.unwrap_or_default();
        lock.packages.retain(|path, entry| {
            !path.is_empty()
                && if flag(entry, "link") {
                    root.join(path).join("package.json").is_file()
                } else {
                    super::isolated::package_path(root, path).join("package.json").is_file()
                }
        });
        return Ok(lock);
    }
    scan(ctx, root)
}

fn scan_using(
    ctx: &Context<'_>,
    root: &Path,
    cached: Option<&BTreeMap<String, Value>>,
) -> Result<Lockfile> {
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
            if !entry.file_type()?.is_dir() && !entry.file_type()?.is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let mut names = Vec::new();
            if name.starts_with('@') && !entry.file_type()?.is_symlink() && entry.path().is_dir() {
                for child in fs::read_dir(entry.path())? {
                    let child = child?;
                    if !child.file_type()?.is_dir() && !child.file_type()?.is_symlink() {
                        continue;
                    }
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
                let mut data = if let Some(data) = cached.and_then(|c| c.get(&location)) {
                    data.clone()
                } else {
                    manifest::read(&path)?.unwrap_or_else(Value::object)
                };
                let workspace = if symlink {
                    ctx.workspaces
                        .borrow()
                        .iter()
                        .find(|w| {
                            w.name == name
                                && fs::canonicalize(&path).ok()
                                    == fs::canonicalize(ctx.source_root.borrow().join(&w.relative))
                                        .ok()
                        })
                        .cloned()
                } else {
                    None
                };
                if let Some(workspace) = &workspace {
                    data.set("link", Value::Bool(true));
                    data.set("tnpmWorkspace", Value::from(workspace.relative.as_str()));
                }
                lock.packages.insert(location.clone(), data);
                if (!symlink || workspace.is_some())
                    && location.matches("node_modules/").count() < 128
                {
                    queue.push_back(location);
                }
                if lock.packages.len() > 50_000 {
                    bail!("installed graph exceeds 50,000 packages");
                }
            }
        }
    }
    Ok(lock)
}

fn link_bins(
    ctx: &Context<'_>,
    root: &Path,
    paths: &BTreeSet<String>,
    lock: &Lockfile,
) -> Result<()> {
    let mut bin_dirs = BTreeSet::from([root.join("node_modules/.bin")]);
    for path in paths {
        let package = root.join(path);
        let modules = if lockfile::name_at(path).starts_with('@') {
            package.parent().unwrap().parent().unwrap()
        } else {
            package.parent().unwrap()
        };
        bin_dirs.insert(modules.join(".bin"));
        if lock.packages.get(path).is_some_and(|e| flag(e, "link")) {
            bin_dirs.insert(package.join("node_modules/.bin"));
        }
    }
    for dir in &bin_dirs {
        if let Ok(relative) = dir.strip_prefix(root) {
            ensure_install_path(ctx, root, &relative.to_string_lossy(), lock)?;
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
    let was_isolated = super::isolated::active(root);
    if was_isolated {
        // An isolated graph cannot be scanned or mutated as a hoisted tree.
        ensure_safe_parents(root, "node_modules")?;
        remove_path(&root.join("node_modules"))?;
    }
    let clean = clean || was_isolated;
    ensure_safe_parents(root, "node_modules")?;
    // Validate all paths before mutating the tree.
    for path in lock.packages.keys().filter(|p| !p.is_empty()) {
        ensure_install_path(ctx, root, path, lock)?;
        if flag(&lock.packages[path], "link") {
            super::sources::workspace_target(ctx, &lock.packages[path])?;
        }
    }
    super::platform::engines(ctx, manifest, "root")?;
    if !super::platform::compatible(manifest) {
        bail!("EBADPLATFORM: unsupported OS/CPU/libc for root");
    }
    let scripts_enabled = !ctx.options.ignore_scripts && !ctx.config.enabled("ignore-scripts");
    let trust = super::trust::Trust::new(manifest, ctx.options)?;
    let mut builds = if clean {
        BTreeMap::new()
    } else {
        build_state(root)
    };
    let mut selected = BTreeSet::new();
    for (path, entry) in &lock.packages {
        if path.is_empty() || ctx.omit(entry) {
            continue;
        }
        if !super::platform::compatible(entry) {
            if flag(entry, "optional") {
                continue;
            }
            bail!("EBADPLATFORM: unsupported OS/CPU/libc for {path}");
        }
        if let Err(error) = super::platform::engines(ctx, entry, path) {
            if flag(entry, "optional") {
                continue;
            }
            return Err(error);
        }
        selected.insert(path.clone());
    }
    retain_reachable(lock, manifest, &mut selected)?;
    if clean {
        let mut workspace_modules = BTreeSet::new();
        for path in &selected {
            let entry = &lock.packages[path];
            if flag(entry, "link") {
                let target = super::sources::workspace_target(ctx, entry)?;
                ensure_safe_parents(&target, "node_modules")?;
                workspace_modules.insert(target.join("node_modules"));
            }
        }
        ctx.progress
            .event("place", "removing existing node_modules for clean install");
        remove_path(&root.join("node_modules"))?;
        for modules in workspace_modules {
            remove_path(&modules)?;
        }
    }
    if scripts_enabled {
        ctx.lifecycle(root, manifest, "preinstall")?;
    }
    fs::create_dir_all(root.join("node_modules"))?;
    ctx.progress.event(
        "prepare",
        &format!("scanning {}", root.join("node_modules").display()),
    );
    let installed = scan(ctx, root)?;
    let mut changed = BTreeSet::new();
    let mut jobs: BTreeSet<_> = selected
        .iter()
        .filter(|path| {
            let entry = &lock.packages[*path];
            if entry
                .str("resolved")
                .is_some_and(|s| s.starts_with("file:"))
            {
                return true;
            }
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
    if !jobs.is_empty() {
        fs::create_dir(&stage)?;
    }
    let result = async {
        let results: Vec<_> = stream::iter(jobs.iter().enumerate().map(|(i, path)| {
            let destination = stage.join(i.to_string());
            async move {
                let entry = &lock.packages[path];
                let result = async {
                    if super::bundles::prepare(ctx, entry, &destination).await?
                        || super::sources::prepare(ctx, entry, &destination).await?
                    {
                        super::bundles::strip_modules(entry, &destination)?;
                        return Ok::<_, anyhow::Error>(());
                    }
                    let package_cache = package_cache_path(ctx, entry, path);
                    if let Some(cached) = package_cache
                        .as_deref()
                        .and_then(|cache| cached_package(cache, entry.str("integrity")))
                    {
                        ctx.progress
                            .event("prepare", &format!("clone package cache {path}"));
                        let dest = destination.clone();
                        let cache_hit =
                            tokio::task::spawn_blocking(move || clone_tree(&cached, &dest)).await?;
                        if cache_hit.is_ok() {
                            super::bundles::strip_modules(entry, &destination)?;
                            return Ok::<_, anyhow::Error>(());
                        }
                        // A stale/partially damaged extracted cache is only an
                        // optimization failure; fall back to the verified tarball.
                        remove_path(&destination)?;
                    }
                    let bytes = tarball(ctx, entry, lockfile::name_at(path)).await?;
                    ctx.progress.event("prepare", &format!("extract {path}"));
                    let dest = destination.clone();
                    let cache = package_cache.clone();
                    let integrity = entry.str("integrity").map(str::to_owned);
                    tokio::task::spawn_blocking(move || -> Result<()> {
                        fs::create_dir_all(&dest)?;
                        unpack(&bytes, &dest).context("could not unpack tarball")?;
                        if let Some(cache) = cache
                            && let Err(error) =
                                cache_extracted_package(&dest, &cache, integrity.as_deref())
                        {
                            crate::util::style::progress::message(format_args!(
                                "tnpm: could not cache extracted package: {error}"
                            ));
                        }
                        Ok(())
                    })
                    .await
                    .context("package extraction task failed")??;
                    super::bundles::strip_modules(entry, &destination)?;
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
                    crate::util::style::progress::message(format_args!(
                        "tnpm: skipped optional {path}: {e:#}"
                    ));
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
            ensure_install_path(ctx, root, &path, lock)?;
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
                ensure_install_path(ctx, root, path, lock)?;
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
            link_bins(ctx, root, &selected, lock)?;
        }
        if scripts_enabled {
            // Nested dependencies first; all binaries already exist before any lifecycle hook.
            let mut order: Vec<_> = selected.iter().cloned().collect();
            order.sort_by_key(|p| std::cmp::Reverse(p.matches("node_modules/").count()));
            for path in &order {
                if !selected.contains(path) {
                    continue;
                }
                let directory = root.join(path);
                if let Some(data) = manifest::read(&directory)? {
                    if !super::trust::Trust::has_scripts(&directory, &data, &lock.packages[path]) {
                        continue;
                    }
                    if !trust.allows(&lock.packages[path], &data) {
                        super::trust::Trust::warn(path, &data);
                        builds.remove(path);
                        continue;
                    }
                    let fingerprint = build_fingerprint(&lock.packages[path], &data, &directory);
                    if !changed.contains(path) && builds.get(path) == Some(&fingerprint) {
                        continue;
                    }
                    for event in ["preinstall", "install", "postinstall", "prepare"] {
                        if event == "prepare" && lock.packages[path].str("tnpmSpec").is_none() {
                            continue;
                        }
                        if let Err(error) = ctx.lifecycle(&directory, &data, event) {
                            if !flag(&lock.packages[path], "optional") {
                                return Err(error);
                            }
                            crate::util::style::progress::message(format_args!(
                                "tnpm: skipped optional {path}: {error:#}"
                            ));
                            selected.remove(path);
                            let previous = selected.clone();
                            retain_reachable(lock, manifest, &mut selected)?;
                            remove_path(&directory)?;
                            for removed in previous.difference(&selected) {
                                remove_path(&root.join(removed))?;
                            }
                            link_bins(ctx, root, &selected, lock)?;
                            break;
                        }
                    }
                    if selected.contains(path) {
                        builds.insert(path.clone(), fingerprint);
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
        if ctx.config.get("node-linker") == Some("isolated") {
            ctx.progress.event("place", "linking isolated dependencies");
            super::isolated::convert(ctx, root, manifest, lock, &selected)?;
        }
        let mut hidden = lock.clone();
        hidden.packages.retain(|path, _| selected.contains(path));
        builds.retain(|path, _| {
            selected.contains(path) && (scripts_enabled || !changed.contains(path))
        });
        let mut hidden_value = hidden.value(manifest, true);
        hidden_value.set("tnpmBuilds", Value::Object(builds.into_iter().collect()));
        write_json(&root.join("node_modules/.tnpm-lock.json"), &hidden_value)?;
        Ok(())
    }
    .await;
    if !jobs.is_empty() {
        let _ = fs::remove_dir_all(&stage);
    }
    result?;
    if ctx.options.command != "update" {
        summary(ctx.options, "installed", changed.len());
    }
    Ok(())
}

pub(super) fn rebuild(ctx: &Context<'_>, root: &Path) -> Result<()> {
    if super::isolated::active(root) {
        bail!("rebuild on isolated installs is not supported yet; run tnpm install to rebuild the layout");
    }
    anyhow::ensure!(
        ctx.options.passthrough.is_empty(),
        "unexpected arguments after --"
    );
    anyhow::ensure!(
        !ctx.options.lock_only,
        "rebuild does not support --package-lock-only"
    );
    let project = manifest::read(root)?.context("rebuild requires package.json")?;
    let trust = super::trust::Trust::new(&project, ctx.options)?;
    let mut installed = snapshot(ctx, root)?;
    // Scanned manifests do not have source identities; retain them from a matching lock.
    if let Some(lock) = Lockfile::read(root)? {
        for (path, data) in &mut installed.packages {
            if let Some(entry) = lock
                .packages
                .get(path)
                .filter(|e| e.str("version") == data.str("version"))
            {
                for key in ["name", "resolved", "integrity", "tnpmSpec"] {
                    if let Some(value) = entry.get(key) {
                        data.set(key, value.clone());
                    }
                }
            }
        }
    }
    let all: BTreeSet<_> = installed.packages.keys().cloned().collect();
    let mut selected = BTreeSet::new();
    if ctx.options.arguments.is_empty() {
        selected = all.clone();
    } else {
        for query in &ctx.options.arguments {
            let mut found = false;
            for (path, entry) in &installed.packages {
                if crate::spec::matches_query(query, path, entry)? {
                    selected.insert(path.clone());
                    found = true;
                }
            }
            anyhow::ensure!(found, "no installed dependency matches: {query}");
        }
    }
    // Reject symlink escapes before relinking binaries or executing any hook.
    for path in &all {
        ensure_install_path(ctx, root, path, &installed)?;
    }
    let mut order: Vec<_> = selected.into_iter().collect();
    order.sort_by_key(|p| std::cmp::Reverse(p.matches("node_modules/").count()));
    link_bins(ctx, root, &all, &installed)?;
    let enabled = !ctx.options.ignore_scripts && !ctx.config.enabled("ignore-scripts");
    let mut builds = build_state(root);
    // A failed rebuild must not leave an old successful-build marker behind.
    remove_path(&root.join("node_modules/.tnpm-lock.json"))?;
    let mut count = 0;
    if enabled {
        for path in order {
            let directory = root.join(&path);
            let data = manifest::read(&directory)?.context("installed dependency disappeared")?;
            let entry = &installed.packages[&path];
            if !super::trust::Trust::has_scripts(&directory, &data, entry) {
                continue;
            }
            if !trust.allows(entry, &data) {
                super::trust::Trust::warn(&path, &data);
                builds.remove(&path);
                continue;
            }
            for event in ["preinstall", "install", "postinstall", "prepare"] {
                if event == "prepare" && entry.get("tnpmSpec").is_none() {
                    continue;
                }
                ctx.lifecycle(&directory, &data, event)?;
            }
            builds.insert(path.clone(), build_fingerprint(entry, &data, &directory));
            count += 1;
        }
    }
    // Re-scan after hooks and refresh binaries without losing successful build state.
    let refreshed = scan(ctx, root)?;
    for path in refreshed.packages.keys() {
        ensure_install_path(ctx, root, path, &refreshed)?;
    }
    let paths = refreshed.packages.keys().cloned().collect();
    link_bins(ctx, root, &paths, &refreshed)?;
    builds.retain(|path, _| refreshed.packages.contains_key(path));
    let mut hidden = refreshed.value(&project, true);
    hidden.set("tnpmBuilds", Value::Object(builds.into_iter().collect()));
    write_json(&root.join("node_modules/.tnpm-lock.json"), &hidden)?;
    summary(ctx.options, "rebuilt", count);
    Ok(())
}

pub(super) fn prune(ctx: &Context<'_>, root: &Path) -> Result<()> {
    if ctx.options.lock_only {
        bail!("prune --package-lock-only is not supported");
    }
    if !ctx.options.all && super::isolated::active(root) {
        bail!("prune on isolated installs is not supported yet; run tnpm install to refresh dependencies");
    }
    if ctx.options.all {
        if !ctx.options.arguments.is_empty() || ctx.options.omit.is_some() || ctx.options.production
        {
            bail!("prune --all cannot be combined with package names or --omit/--production");
        }
        let modules = root.join("node_modules");
        let count = if fs::symlink_metadata(&modules)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            scan(ctx, root)?.packages.len()
        } else {
            0
        };
        remove_path(&modules)?;
        summary(ctx.options, "removed", count);
        return Ok(());
    }
    let manifest = manifest::read(root)?.context("package.json not found")?;
    let (manifest, _) = super::sources::effective(root, &manifest)?;
    let old = Lockfile::read(root)?;
    let mut installed = scan(ctx, root)?;
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
            ensure_install_path(ctx, root, path, &installed)?;
            remove_path(&root.join(path))?;
            removed += 1;
        } else {
            retained.insert(path.clone());
        }
    }
    // Parent removal may also remove previously retained nested extras.
    retained.retain(|path| root.join(path).join("package.json").is_file());
    if !ctx.options.lock_only {
        link_bins(ctx, root, &retained, &installed)?;
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
    remove_path(&root.join("node_modules/.tnpm-lock.json"))?;
    summary(ctx.options, "removed", removed);
    Ok(())
}
