//! Preserve bundled dependency manifests in the lock graph, while extracting each
//! physical package separately so pruning, bin links, and platform checks still work.
use super::{context::Context, resolver::descriptor, sources, store};
use crate::{
    json::Value,
    lockfile::{self, Lockfile},
    manifest,
};
use anyhow::{Context as _, Result, ensure};
use std::{fs, path::Path};

pub(super) async fn hydrate(ctx: &Context<'_>, mut data: Value) -> Result<Value> {
    if data.get("link") == Some(&Value::Bool(true)) {
        return Ok(data);
    }
    let bundled = data
        .get("bundleDependencies")
        .or_else(|| data.get("bundledDependencies"));
    if !matches!(bundled, Some(Value::Array(v)) if !v.is_empty())
        && bundled != Some(&Value::Bool(true))
    {
        return Ok(data);
    }
    let name = data.str("name").context("bundled package has no name")?;
    let entry = descriptor(&data)?;
    let temp = tempfile::tempdir()?;
    let directory = temp.path().join("package");
    let cache = store::package_cache_path(ctx, &entry, name);
    if let Some(cached) = cache
        .as_deref()
        .and_then(|p| store::cached_package(p, entry.str("integrity")))
    {
        store::clone_tree(&cached, &directory)?;
    } else if !sources::prepare(ctx, &entry, &directory).await? {
        let bytes = store::tarball(ctx, &entry, name).await?;
        fs::create_dir(&directory)?;
        store::unpack(&bytes, &directory)?;
        if let Some(cache) = cache {
            store::cache_extracted_package(&directory, &cache, entry.str("integrity"))?;
        }
    }
    let mut packages = Value::object();
    fn scan(
        directory: &Path,
        relative: &str,
        parent: &Value,
        out: &mut Value,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth < 128, "bundled dependency nesting exceeds 128 levels");
        let modules = directory.join(relative).join("node_modules");
        if !modules.is_dir() {
            return Ok(());
        }
        let mut names = Vec::new();
        for entry in fs::read_dir(&modules)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            ensure!(
                entry.file_type()?.is_dir(),
                "bundled dependency is not a directory"
            );
            if name.starts_with('@') {
                for child in fs::read_dir(entry.path())? {
                    names.push(format!("{name}/{}", child?.file_name().to_string_lossy()));
                }
            } else {
                names.push(name);
            }
        }
        names.sort();
        for name in names {
            ensure!(
                lockfile::valid_name(&name),
                "invalid bundled dependency name"
            );
            let location = lockfile::child(relative, &name);
            let data = manifest::read(&directory.join(&location))?
                .context("bundled dependency has no package.json")?;
            let mut data = data;
            let mut dist = Value::object();
            dist.set(
                "tarball",
                parent
                    .get("resolved")
                    .context("bundle parent has no resolved source")?
                    .clone(),
            );
            if let Some(integrity) = parent.get("integrity") {
                dist.set("integrity", integrity.clone());
            }
            data.set("dist", dist);
            let mut entry = descriptor(&data)?;
            entry.set("tnpmBundlePath", Value::from(location.as_str()));
            entry.set("inBundle", Value::Bool(true));
            entry.set(
                "tnpmBundleRoot",
                lockfile::copy_fields(parent, &["name", "version", "resolved", "integrity"]),
            );
            out.set(&location, entry);
            scan(directory, &location, parent, out, depth + 1)?;
        }
        Ok(())
    }
    scan(&directory, "", &entry, &mut packages, 0)?;
    let required = match bundled {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::string)
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        _ => lockfile::dependencies(&data, false)
            .into_iter()
            .filter(|d| !d.peer)
            .map(|d| d.name)
            .collect(),
    };
    for name in required {
        if data
            .get("optionalDependencies")
            .and_then(|d| d.get(&name))
            .is_some()
        {
            continue;
        }
        ensure!(
            packages.get(&lockfile::child("", &name)).is_some(),
            "declared bundled dependency is absent from archive: {name}"
        );
    }
    data.set("tnpmBundled", packages);
    Ok(data)
}

pub(super) fn insert(
    lock: &mut Lockfile,
    root: &Value,
    location: &str,
    entry: &Value,
) -> Result<Vec<(String, lockfile::Dependency)>> {
    let mut edges = Vec::new();
    for (relative, data) in entry
        .get("tnpmBundled")
        .map(Value::members)
        .unwrap_or_default()
    {
        ensure!(
            lockfile::valid_location(relative),
            "unsafe bundled dependency location"
        );
        let path = format!("{location}/{relative}");
        let parent = if lockfile::parent(&path) == location {
            Some(entry)
        } else {
            lock.packages.get(lockfile::parent(&path))
        };
        let name = lockfile::name_at(&path);
        if let Some(parent) = parent {
            for dep in lockfile::dependencies(parent, false)
                .iter()
                .filter(|d| d.name == name && !d.peer)
            {
                ensure!(
                    crate::spec::satisfies(name, &dep.spec, data) != Some(false),
                    "EOVERRIDE: bundled dependency {name} cannot be replaced by {}",
                    dep.spec
                );
            }
        }
        let mut data = data.clone();
        super::overrides::apply(root, parent, name, &mut data)?;
        edges.extend(
            lockfile::dependencies(&data, false)
                .into_iter()
                .map(|dep| (path.clone(), dep)),
        );
        lock.packages.insert(path, data);
    }
    Ok(edges)
}

pub(super) async fn prepare(ctx: &Context<'_>, entry: &Value, destination: &Path) -> Result<bool> {
    let Some(relative) = entry.str("tnpmBundlePath") else {
        return Ok(false);
    };
    ensure!(
        lockfile::valid_location(relative),
        "unsafe bundled dependency path"
    );
    if let Some(root) = entry.get("tnpmBundleRoot")
        && let Some(cache) =
            store::package_cache_path(ctx, root, root.str("name").unwrap_or("bundle"))
        && let Some(package) = store::cached_package(&cache, root.str("integrity"))
    {
        store::clone_tree(&package.join(relative), destination)?;
        return Ok(true);
    }
    let temp = tempfile::tempdir()?;
    let parent = temp.path().join("parent");
    let mut archive_entry = entry.clone();
    archive_entry.remove("tnpmBundlePath");
    if !sources::prepare(ctx, &archive_entry, &parent).await? {
        let bytes = store::tarball(ctx, entry, entry.str("name").unwrap_or("bundle")).await?;
        fs::create_dir(&parent)?;
        store::unpack(&bytes, &parent)?;
    }
    store::clone_tree(&parent.join(relative), destination)?;
    Ok(true)
}

pub(super) fn strip_modules(entry: &Value, destination: &Path) -> Result<()> {
    if (entry.get("tnpmBundled").is_some() || entry.get("tnpmBundlePath").is_some())
        && destination.join("node_modules").exists()
    {
        fs::remove_dir_all(destination.join("node_modules"))?;
    }
    Ok(())
}
