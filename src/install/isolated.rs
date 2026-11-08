//! pnpm-style isolated linker. Package payloads live in a project-local virtual
//! store; only declared dependencies are exposed by directory links.
use super::{context::Context, flag, store};
use crate::{
    json::Value,
    lockfile::{self, Lockfile},
    manifest, scripts,
};
use anyhow::{Context as _, Result, bail};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::path::Component;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = "node_modules/.tnpm-layout";

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
        if metadata.file_type().is_symlink()
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
    std::os::windows::fs::symlink_dir(target, destination).with_context(|| {
        format!(
            "could not link {}; enable Windows Developer Mode to allow directory symlinks",
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
                use std::os::unix::fs::{PermissionsExt, symlink};
                fs::set_permissions(
                    &source,
                    fs::Permissions::from_mode(fs::metadata(&source)?.permissions().mode() | 0o111),
                )?;
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
    // Mark before relocating anything: a failed conversion is discarded on the
    // next install rather than treating half-moved packages as hoisted.
    fs::write(root.join(MARKER), "isolated\n")?;
    let mut order: Vec<_> = selected
        .iter()
        .filter(|path| !flag(&lock.packages[*path], "link"))
        .collect();
    // Move descendants before parents, avoiding relocation of nested node_modules.
    order.sort_by_key(|path| {
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
        fs::create_dir_all(
            destination
                .parent()
                .context("virtual package has no parent")?,
        )?;
        fs::rename(&source, &destination)
            .with_context(|| format!("could not move {path} into isolated store"))?;
    }
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
