//! pnpm-style isolated linker. Package payloads live in a project-local virtual
//! store; only declared dependencies are exposed by directory links.
use super::{CONCURRENCY, context::Context, flag, hardlinks, store};
use crate::{
    json::Value,
    lockfile::{self, Lockfile},
    manifest, scripts,
    util::style::progress::{self, Phase},
};
use anyhow::{Context as _, Result, bail, ensure};
use futures_util::{StreamExt, stream};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::path::Component;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = "node_modules/.tnpm-layout";

fn index_key(entry: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let mut fields: Vec<_> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), canonical(v)))
                    .collect();
                fields.sort_by(|a, b| a.0.cmp(&b.0));
                Value::Object(fields)
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            value => value.clone(),
        }
    }
    let identity =
        lockfile::copy_fields(entry, &["name", "version", "resolved", "integrity", "bin"]);
    const_hex::encode(Sha256::digest(canonical(&identity).pretty().as_bytes()))
}

pub(super) async fn try_install(
    ctx: &Context<'_>,
    root: &Path,
    manifest: &Value,
    lock: &Lockfile,
    selected: &mut BTreeSet<String>,
    scripts_enabled: bool,
) -> Result<bool> {
    if manifest.get("workspaces").is_some()
        || selected.iter().any(|path| {
            let entry = &lock.packages[path];
            flag(entry, "link")
                || entry.get("tnpmSpec").is_some()
                || entry.get("tnpmBundled").is_some()
                || entry.get("tnpmBundlePath").is_some()
                || entry.get("bundleDependencies").is_some()
                || entry.get("bundledDependencies").is_some()
                || store::package_cache_path(ctx, entry, path).is_none()
        })
    {
        return Ok(false);
    }
    if scripts_enabled
        && ([
            "preinstall",
            "install",
            "postinstall",
            "prepublish",
            "preprepare",
            "prepare",
            "postprepare",
        ]
        .iter()
        .any(|event| manifest.get("scripts").and_then(|s| s.str(event)).is_some())
            || scripts::implicit_native(root, manifest))
    {
        return Ok(false);
    }
    let trust = super::trust::Trust::new(manifest, ctx.options)?;
    super::tree::ensure_safe_parents(root, "node_modules/.tnpm")?;
    super::tree::ensure_safe_parents(root, MARKER)?;
    let store = Path::new(ctx.config.get("store-dir").context("missing store-dir")?);
    super::tree::ensure_safe_parents(store, "files-index-v1")?;
    let stage = tempfile::Builder::new()
        .prefix(".tnpm-isolated-")
        .tempdir_in(root)?;
    let mut prepared = Vec::new();
    let paths: Vec<_> = selected.iter().cloned().collect();
    ctx.progress
        .start_many(Phase::Preparing, paths.iter().map(String::as_str));
    let stage_path = stage.path();
    let trust = &trust;
    let mut jobs = stream::iter(paths.iter().enumerate().map(|(n, path)| async move {
        let result = async {
            let entry = &lock.packages[path];
            let key = index_key(entry);
            let index_path = store.join("files-index-v1").join(format!("{key}.json"));
            super::tree::ensure_safe_parents(store, &format!("files-index-v1/{key}.json"))?;
            let old = fs::symlink_metadata(&index_path)
                .ok()
                .filter(|m| m.is_file() && !super::tree::is_link(m))
                .and_then(|_| fs::read_to_string(&index_path).ok())
                .and_then(|s| Value::parse(&s).ok())
                .filter(|v| {
                    v.str("identity") == Some(key.as_str())
                        && v.str("version") == Some("1")
                        && matches!(v.get("hooks"), Some(Value::Bool(_)))
                        && matches!(v.get("manifest"), Some(Value::Object(_)))
                });
            if let Some(index) = &old
                && scripts_enabled
                && flag(index, "hooks")
            {
                let data = index
                    .get("manifest")
                    .context("package index has no manifest")?;
                if trust.allows(entry, data) {
                    return Ok(None);
                }
                super::trust::Trust::warn(path, data);
            }
            let destination = stage_path.join(n.to_string());
            if let Some(index) = &old
                && hardlinks::restore(index, store, &destination)?
            {
                return Ok(Some(destination));
            }
            let payload = stage_path.join(format!("payload-{n}"));
            prepare_payload(ctx, entry, path, &payload).await?;
            let data = manifest::read(&payload)?.context("package has no manifest")?;
            let hooks = super::trust::Trust::has_scripts(&payload, &data, entry);
            if hooks && scripts_enabled {
                if trust.allows(entry, &data) {
                    return Ok::<_, anyhow::Error>(None);
                }
                super::trust::Trust::warn(path, &data);
            }
            executable_bins(&payload, entry, &data)?;
            let mut index = hardlinks::index_package(&payload, store, old.as_ref())?;
            index.set("identity", Value::from(key.as_str()));
            index.set("version", Value::from("1"));
            index.set("manifest", data);
            index.set("hooks", Value::Bool(hooks));
            fs::create_dir_all(index_path.parent().unwrap())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    index_path.parent().unwrap(),
                    fs::Permissions::from_mode(0o700),
                )?;
            }
            crate::fs::write_json(&index_path, &index)?;
            ensure!(
                hardlinks::restore(&index, store, &destination)?,
                "store changed while linking package"
            );
            Ok(Some(destination))
        }
        .await;
        (path, result)
    }))
    .buffer_unordered(CONCURRENCY);
    while let Some((path, result)) = jobs.next().await {
        ctx.progress.complete(Phase::Preparing, path);
        let entry = &lock.packages[path];
        match result {
            Ok(Some(destination)) => prepared.push((path.clone(), destination)),
            Ok(None) => return Ok(false),
            Err(error) if flag(entry, "optional") => {
                progress::single_line_message(format_args!(
                    "tnpm: skipped optional {path}: {error:#}"
                ));
                selected.remove(path);
            }
            Err(error) => return Err(error).with_context(|| format!("could not prepare {path}")),
        }
    }
    drop(jobs);
    let used = lock.reachable_selected(manifest, selected)?;
    selected.retain(|p| used.contains(p));
    super::tree::remove_path(&root.join("node_modules"))?;
    fs::create_dir_all(root.join("node_modules/.tnpm"))?;
    fs::write(root.join(MARKER), "isolated\n")?;
    ctx.progress.start_many(
        Phase::Linking,
        prepared
            .iter()
            .filter(|(path, _)| selected.contains(path))
            .map(|(path, _)| path.as_str()),
    );
    for (path, source) in prepared {
        if !selected.contains(&path) {
            continue;
        }
        let destination = package_path(root, &path);
        fs::create_dir_all(destination.parent().unwrap())?;
        fs::rename(source, destination)?;
        ctx.progress.complete(Phase::Linking, &path);
    }
    finish_links(ctx, root, manifest, lock, selected)?;
    let mut hidden = lock.clone();
    hidden.packages.retain(|path, _| selected.contains(path));
    let mut hidden_value = hidden.value(manifest, true);
    hidden_value.set("tnpmBuilds", Value::object());
    crate::fs::write_json(&root.join("node_modules/.tnpm-lock.json"), &hidden_value)?;
    if ctx.options.command != "update" {
        super::summary(ctx.options, "installed", selected.len());
    }
    Ok(true)
}

async fn prepare_payload(
    ctx: &Context<'_>,
    entry: &Value,
    path: &str,
    destination: &Path,
) -> Result<()> {
    let cache = store::package_cache_path(ctx, entry, path);
    if let Some(cached) = cache
        .as_deref()
        .and_then(|p| store::cached_package(p, entry.str("integrity")))
    {
        let clones = ctx.clones.clone();
        let dest = destination.to_owned();
        if tokio::task::spawn_blocking(move || clones.clone_tree(&cached, &dest))
            .await?
            .is_ok()
        {
            return Ok(());
        }
        super::tree::remove_path(destination)?;
    }
    let bytes = store::tarball(ctx, entry).await?;
    fs::create_dir(destination)?;
    let dest = destination.to_owned();
    let clones = ctx.clones.clone();
    let integrity = entry.str("integrity").map(str::to_owned);
    tokio::task::spawn_blocking(move || -> Result<()> {
        store::unpack(&bytes, &dest)?;
        if let Some(cache) = cache
            && let Err(error) =
                store::cache_extracted_package(&dest, &cache, integrity.as_deref(), &clones)
        {
            crate::util::style::progress::message(format_args!(
                "tnpm: could not cache extracted package: {error}"
            ));
        }
        Ok(())
    })
    .await??;
    Ok(())
}

fn executable_bins(directory: &Path, entry: &Value, manifest: &Value) -> Result<()> {
    let data = if entry.get("bin").is_some() {
        entry
    } else {
        manifest
    };
    for (name, target) in scripts::bins(data) {
        ensure!(
            !name.is_empty()
                && !name.contains(['/', '\\'])
                && !matches!(name.as_str(), "." | "..")
                && store::safe_relative(Path::new(&target)),
            "unsafe bin entry"
        );
        super::tree::ensure_safe_parents(directory, &target)?;
        let source = directory.join(target);
        if !source.is_file() {
            continue;
        }
        ensure!(
            fs::canonicalize(&source)?.starts_with(fs::canonicalize(directory)?),
            "bin target escapes package"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &source,
                fs::Permissions::from_mode(fs::metadata(&source)?.permissions().mode() | 0o111),
            )?;
        }
    }
    Ok(())
}

pub(super) fn active(root: &Path) -> bool {
    let path = root.join(MARKER);
    fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
        && fs::read_to_string(path).is_ok_and(|value| value == "isolated\n")
}

pub(super) fn package_path(root: &Path, path: &str) -> PathBuf {
    let digest = Sha256::digest(path.as_bytes());
    let mut id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        use std::fmt::Write;
        write!(&mut id, "{byte:02x}").expect("writing to string");
    }
    root.join("node_modules/.tnpm")
        .join(id)
        .join("node_modules")
        .join(lockfile::name_at(path))
}

#[cfg(unix)]
fn relative(from: &Path, target: &Path) -> PathBuf {
    let from: Vec<Component<'_>> = from.components().collect();
    let target: Vec<Component<'_>> = target.components().collect();
    let common = from.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..from.len() {
        path.push("..");
    }
    for component in &target[common..] {
        path.push(component.as_os_str());
    }
    path
}

fn directory_link(target: &Path, destination: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if super::tree::is_link(&metadata)
            && let (Ok(actual), Ok(expected)) =
                (fs::canonicalize(destination), fs::canonicalize(target))
            && actual == expected
        {
            return Ok(());
        }
        bail!(
            "refusing to replace existing dependency path: {}",
            destination.display()
        );
    }
    fs::create_dir_all(destination.parent().context("link has no parent")?)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(relative(destination.parent().unwrap(), target), destination)?;
    #[cfg(windows)]
    super::junction::create(target, destination).with_context(|| {
        format!(
            "could not create directory junction: {}",
            destination.display()
        )
    })?;
    Ok(())
}

fn binaries(modules: &Path, dependencies: &[(String, PathBuf)]) -> Result<()> {
    let bin = modules.join(".bin");
    super::tree::remove_path(&bin)?;
    fs::create_dir_all(&bin)?;
    for (package_name, package) in dependencies {
        let Some(data) = manifest::read(package)? else {
            continue;
        };
        for (name, entry) in scripts::bins(&data) {
            if name.is_empty()
                || name.contains(['/', '\\'])
                || matches!(name.as_str(), "." | "..")
                || !store::safe_relative(Path::new(&entry))
            {
                bail!("unsafe bin entry: {name} -> {entry}");
            }
            let source = package.join(&entry);
            if !source.is_file() {
                continue;
            }
            if !fs::canonicalize(&source)?.starts_with(fs::canonicalize(package)?) {
                bail!("bin target escapes package: {}", source.display());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                let target = Path::new("..").join(package_name).join(&entry);
                let destination = bin.join(&name);
                if fs::symlink_metadata(&destination).is_err() {
                    symlink(target, destination)?;
                }
            }
            #[cfg(windows)]
            {
                let destination = bin.join(format!("{name}.cmd"));
                if !destination.exists() {
                    fs::write(
                        destination,
                        format!("@echo off\r\nnode \"{}\" %*\r\n", source.display()),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn link_dependencies(
    ctx: &Context<'_>,
    root: &Path,
    from: &str,
    data: &Value,
    lock: &Lockfile,
    selected: &BTreeSet<String>,
    modules: &Path,
) -> Result<()> {
    fs::create_dir_all(modules)?;
    let mut links = Vec::new();
    for dep in lockfile::dependencies(data, from.is_empty()) {
        let Some(location) = lock.resolve_dependency(from, &dep) else {
            continue;
        };
        if !selected.contains(&location) {
            continue;
        }
        let entry = &lock.packages[&location];
        let package = if flag(entry, "link") {
            super::sources::workspace_target(ctx, entry)?
        } else {
            package_path(root, &location)
        };
        directory_link(&package, &modules.join(&dep.name))?;
        links.push((dep.name, package));
    }
    binaries(modules, &links)
}

pub(super) fn convert(
    ctx: &Context<'_>,
    root: &Path,
    manifest: &Value,
    lock: &Lockfile,
    selected: &BTreeSet<String>,
) -> Result<()> {
    // Refuse pre-existing links into untrusted directories and do not follow a
    // symlinked marker when writing it.
    super::tree::ensure_safe_parents(root, "node_modules/.tnpm")?;
    super::tree::ensure_safe_parents(root, MARKER)?;
    // Mark before relocating anything: a failed conversion is discarded on the
    // next install rather than treating half-moved packages as hoisted.
    fs::write(root.join(MARKER), "isolated\n")?;
    let store = Path::new(ctx.config.get("store-dir").context("missing store-dir")?);
    let mut order: Vec<_> = selected
        .iter()
        .filter(|path| !flag(&lock.packages[*path], "link"))
        .collect();
    // Move descendants before parents, avoiding relocation of nested node_modules.
    order.sort_by_cached_key(|path| {
        std::cmp::Reverse((path.matches("/node_modules/").count(), path.len()))
    });
    for path in order {
        let source = root.join(path);
        let metadata = fs::symlink_metadata(&source)
            .with_context(|| format!("installed package missing: {}", source.display()))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "refusing to relocate non-directory package: {}",
                source.display()
            );
        }
        super::tree::remove_path(&source.join("node_modules"))?;
        let destination = package_path(root, path);
        super::tree::ensure_safe_parents(root, &destination.strip_prefix(root)?.to_string_lossy())?;
        fs::create_dir_all(
            destination
                .parent()
                .context("virtual package has no parent")?,
        )?;
        fs::rename(&source, &destination)
            .with_context(|| format!("could not move {path} into isolated store"))?;
        // Publish each final (including script-generated) file by content hash.
        // The virtual store contains hardlinks, never CoW clones/copies.
        super::hardlinks::link_package(&destination, store)?;
    }
    // Workspace dependencies may have been installed inside the symlink target.
    // Their payloads have already been relocated above; replace the old hoisted
    // directories before adding isolated dependency links.
    for path in selected {
        let entry = &lock.packages[path];
        if flag(entry, "link") {
            let target = super::sources::workspace_target(ctx, entry)?;
            super::tree::ensure_safe_parents(&target, "node_modules")?;
            super::tree::remove_path(&target.join("node_modules"))?;
        }
    }
    finish_links(ctx, root, manifest, lock, selected)
}

fn finish_links(
    ctx: &Context<'_>,
    root: &Path,
    manifest: &Value,
    lock: &Lockfile,
    selected: &BTreeSet<String>,
) -> Result<()> {
    // Every package has its own node_modules: unrelated hoisted dependencies
    // cannot leak into its resolution graph.
    for path in selected {
        let entry = &lock.packages[path];
        let target = if flag(entry, "link") {
            super::sources::workspace_target(ctx, entry)?
        } else {
            package_path(root, path)
        };
        link_dependencies(
            ctx,
            root,
            path,
            entry,
            lock,
            selected,
            &target.join("node_modules"),
        )?;
    }
    // Only direct project dependencies are visible from the project root.
    link_dependencies(
        ctx,
        root,
        "",
        manifest,
        lock,
        selected,
        &root.join("node_modules"),
    )?;
    Ok(())
}
