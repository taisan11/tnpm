//! Tarball fetching, integrity checks, safe extraction, and copy-on-write caching.
use super::context::Context;
use crate::{fs::unique, json::Value};
use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256, Sha512};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

pub(super) fn verify(bytes: &[u8], integrity: Option<&str>) -> Result<()> {
    let Some(integrity) = integrity else {
        return Ok(());
    };
    // As in SRI, prefer the strongest algorithm present; a weak match cannot mask a bad SHA-512.
    for algorithm in ["sha512", "sha256", "sha1"] {
        let hashes: Vec<_> = integrity
            .split_whitespace()
            .filter_map(|s| s.split_once('-'))
            .filter(|(a, _)| *a == algorithm)
            .map(|(_, s)| s.split('?').next().unwrap_or(s))
            .collect();
        if hashes.is_empty() {
            continue;
        }
        let actual = match algorithm {
            "sha512" => STANDARD.encode(Sha512::digest(bytes)),
            "sha256" => STANDARD.encode(Sha256::digest(bytes)),
            _ => STANDARD.encode(sha1::Sha1::digest(bytes)),
        };
        ensure!(
            hashes.contains(&actual.as_str()),
            "EINTEGRITY: tarball checksum mismatch"
        );
        return Ok(());
    }
    bail!("unsupported tarball integrity algorithm")
}

pub(super) fn package_cache_path(ctx: &Context<'_>, entry: &Value, path: &str) -> Option<PathBuf> {
    if ctx.options.no_cache {
        return None;
    }
    // Without an integrity digest, name and version cannot identify the bytes:
    // another registry may serve different contents under the same coordinates.
    entry.str("integrity").filter(|value| !value.is_empty())?;
    let resolved = entry.str("resolved")?;
    let url = reqwest::Url::parse(resolved).ok()?;
    if ctx.config.authenticated(&url) {
        return None;
    }
    let name = entry
        .str("name")
        .unwrap_or_else(|| crate::lockfile::name_at(path));
    let version = entry.str("version")?;
    if !crate::lockfile::valid_name(name)
        || version.is_empty()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return None;
    }
    Some(
        dirs::home_dir()?
            .join(".tnpm/cache/packages-v2")
            .join(format!("{name}@{version}")),
    )
}

pub(super) fn cached_package(cache: &Path, integrity: Option<&str>) -> Option<PathBuf> {
    let marker = fs::symlink_metadata(cache.join(".complete")).ok()?;
    let package = cache.join("package");
    let directory = fs::symlink_metadata(&package).ok()?;
    let manifest = fs::symlink_metadata(package.join("package.json")).ok()?;
    (marker.is_file()
        && !marker.file_type().is_symlink()
        && fs::read(cache.join(".complete")).ok()? == integrity.unwrap_or("").as_bytes()
        && directory.is_dir()
        && !directory.file_type().is_symlink()
        && manifest.is_file()
        && !manifest.file_type().is_symlink())
    .then_some(package)
}

// Prefer filesystem copy-on-write clones: they share data blocks initially,
// while writes from lifecycle scripts remain isolated from the package cache.
#[cfg(target_os = "macos")]
fn clone_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
        -> libc::c_int;
    }
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if unsafe { clonefile(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
fn clone_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    use std::os::fd::AsRawFd;
    let source_file = fs::File::open(source)?;
    let destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    // Linux FICLONE: clone file data using the destination filesystem's
    // copy-on-write implementation (e.g. Btrfs, XFS, or APFS-compatible FUSE).
    const FICLONE: libc::c_ulong = 0x4004_9409;
    let result = unsafe {
        libc::ioctl(
            destination_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };
    drop(destination_file);
    if result == 0 {
        fs::set_permissions(destination, fs::metadata(source)?.permissions())?;
        Ok(true)
    } else {
        drop(source_file);
        fs::remove_file(destination)?;
        Ok(false)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn clone_file(_source: &Path, _destination: &Path) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(target_os = "macos")]
fn validate_cloned_tree(path: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            validate_cloned_tree(&entry.path())?;
        } else if !kind.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "package cache contains a link or special file",
            ));
        }
    }
    Ok(())
}

pub(super) fn clone_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if fs::symlink_metadata(source).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && clone_file(source, destination)?
    {
        // clonefile also clones symlinks; validate before the package can be placed.
        if let Err(error) = validate_cloned_tree(destination) {
            let _ = fs::remove_dir_all(destination);
            return Err(error);
        }
        return Ok(());
    }
    clone_tree_files(source, destination)
}

fn clone_tree_files(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            clone_tree_files(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "package cache contains a link or special file",
        ));
    }
    if !clone_file(source, destination)? {
        fs::copy(source, destination)?;
    }
    Ok(())
}

pub(super) fn cache_extracted_package(
    source: &Path,
    cache: &Path,
    integrity: Option<&str>,
) -> Result<()> {
    let parent = cache.parent().context("package cache path has no parent")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let temporary = parent.join(format!(".{}-{}", std::process::id(), unique()));
    let result = (|| -> Result<()> {
        fs::create_dir(&temporary)?;
        let package = temporary.join("package");
        clone_tree(source, &package)?;
        private_cache_write(
            &temporary.join(".complete"),
            integrity.unwrap_or("").as_bytes(),
        )?;
        match fs::rename(&temporary, cache) {
            Ok(()) => Ok(()),
            Err(error) if cache.exists() => {
                // Another concurrent install may have published the same key.
                let _ = fs::remove_dir_all(&temporary);
                if cached_package(cache, integrity).is_some() {
                    Ok(())
                } else {
                    Err(error.into())
                }
            }
            Err(error) => Err(error.into()),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

pub(super) async fn tarball(ctx: &Context<'_>, entry: &Value, name: &str) -> Result<Vec<u8>> {
    let resolved = entry
        .str("resolved")
        .context("lockfile package has no resolved tarball URL")?;
    let url = reqwest::Url::parse(resolved)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("only HTTP(S) tarballs without URL credentials are supported");
    }
    let integrity = entry.str("integrity");
    if let Some(bytes) = ctx.source_tarballs.borrow().get(resolved) {
        verify(bytes, integrity)?;
        return Ok(bytes.clone());
    }
    if let Some(cache) = source_tarball_cache(ctx, entry)
        && let Ok(bytes) = fs::read(cache)
    {
        verify(&bytes, integrity)?;
        return Ok(bytes);
    }
    let authorization = ctx.config.authorization(&url)?;
    if ctx.offline() {
        bail!("ENOTCACHED: tarball {resolved}");
    }
    let mut request = ctx
        .client(&url, true)?
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if ctx.options.no_cache {
        request = request.header(reqwest::header::CACHE_CONTROL, "no-cache, no-store");
    }
    if let Some(authorization) = authorization {
        request = request.header(reqwest::header::AUTHORIZATION, authorization);
    }
    ctx.progress
        .event("prepare", &format!("HTTP GET tarball {name}"));
    let response = request.send().await?.error_for_status()?;
    if response.status() != reqwest::StatusCode::OK {
        bail!("unexpected tarball HTTP status: {}", response.status());
    }
    let bytes = response.bytes().await?.to_vec();
    verify(&bytes, integrity)?;
    if entry
        .str("tnpmSpec")
        .is_some_and(super::sources::is_tarball)
    {
        remember_source_tarball(ctx, entry, &bytes)?;
    }
    Ok(bytes)
}

fn source_tarball_cache(ctx: &Context<'_>, entry: &Value) -> Option<PathBuf> {
    if ctx.options.no_cache
        || !entry
            .str("tnpmSpec")
            .is_some_and(super::sources::is_tarball)
    {
        return None;
    }
    let resolved = entry.str("resolved")?;
    let url = reqwest::Url::parse(resolved).ok()?;
    if ctx.config.authenticated(&url) {
        return None;
    }
    let integrity = entry.str("integrity")?;
    let key = format!("{resolved}\n{integrity}");
    Some(
        dirs::home_dir()?
            .join(".tnpm/cache/source-tarballs-v1")
            .join(format!("{:x}.tgz", Sha256::digest(key.as_bytes()))),
    )
}

pub(super) fn remember_source_tarball(
    ctx: &Context<'_>,
    entry: &Value,
    bytes: &[u8],
) -> Result<()> {
    verify(bytes, entry.str("integrity"))?;
    if let Some(cache) = source_tarball_cache(ctx, entry)
        && let Err(error) = private_cache_write(&cache, bytes)
    {
        crate::util::style::progress::message(format_args!(
            "tnpm: could not cache source tarball: {error}"
        ));
    }
    if let Some(resolved) = entry.str("resolved") {
        ctx.source_tarballs
            .borrow_mut()
            .insert(resolved.to_owned(), bytes.to_vec());
    }
    Ok(())
}

fn private_cache_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("cache path has no parent")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let temp = parent.join(format!(".{}-{}", std::process::id(), unique()));
    let result = (|| -> Result<()> {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&temp)?.write_all(bytes)?;
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(super) fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && !path.to_string_lossy().contains('\\')
}

pub(super) fn unpack(bytes: &[u8], dest: &Path) -> Result<()> {
    let reader: Box<dyn Read> = if bytes.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::GzDecoder::new(bytes))
    } else {
        Box::new(Cursor::new(bytes))
    };
    let mut archive = tar::Archive::new(reader);
    archive.set_preserve_permissions(false);
    // Archives commonly contain many files in the same directory. Avoid
    // repeating create_dir_all (and its metadata syscalls) for each file.
    let mut created_parents = HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if !safe_relative(&path) {
            bail!("unsafe tarball path: {}", path.display());
        }
        let normalized = path.strip_prefix("./").unwrap_or(&path);
        let mut components = normalized.components();
        components.next(); // npm tarballs have one top-level directory (usually `package/`).
        let relative = components.as_path();
        if relative.as_os_str().is_empty() {
            continue;
        }
        if !safe_relative(relative) {
            bail!("unsafe tarball path");
        }
        let kind = entry.header().entry_type();
        // Registry tarballs do not need links/devices. Reject rather than risk escapes.
        if !(kind.is_file() || kind.is_dir()) {
            bail!(
                "unsupported tarball entry (links/devices): {}",
                path.display()
            );
        }
        let target = dest.join(relative);
        if let Some(parent) = target.parent()
            && created_parents.insert(parent.to_owned())
        {
            fs::create_dir_all(parent)?;
        }
        entry.unpack(target)?;
    }
    if !dest.join("package.json").is_file() {
        bail!("tarball contains no package.json");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn integrity_prefers_strongest_hash() {
        let data = b"hello";
        let sha1 = STANDARD.encode(sha1::Sha1::digest(data));
        let sha512 = STANDARD.encode(Sha512::digest(data));
        assert!(verify(data, Some(&format!("sha512-{sha512}"))).is_ok());
        assert!(verify(data, Some(&format!("sha1-{sha1} sha512-wrong"))).is_err());
        assert!(verify(b"bad", Some(&format!("sha512-{sha512}"))).is_err());
    }
    #[test]
    fn rejects_unsafe_relative_paths() {
        for path in ["../x", "/x", "a/../../x", "a\\b"] {
            assert!(!safe_relative(Path::new(path)));
        }
        assert!(safe_relative(Path::new("bin/cli.js")));
    }

    #[test]
    fn cloned_package_files_are_isolated_from_the_cache() {
        let root =
            std::env::temp_dir().join(format!("tnpm-clone-{}-{}", std::process::id(), unique()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("package.json"), b"{\"name\":\"fixture\"}").unwrap();
        fs::write(source.join("nested/file.js"), b"original").unwrap();

        clone_tree(&source, &destination).unwrap();
        fs::write(destination.join("nested/file.js"), b"modified").unwrap();

        assert_eq!(
            fs::read(source.join("nested/file.js")).unwrap(),
            b"original"
        );
        assert_eq!(
            fs::read(destination.join("nested/file.js")).unwrap(),
            b"modified"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn cloned_package_rejects_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "tnpm-clone-link-{}-{}",
            std::process::id(),
            unique()
        ));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(&source).unwrap();
        std::os::unix::fs::symlink("/etc", source.join("link")).unwrap();
        assert!(clone_tree(&source, &destination).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_tarball_links_and_parent_traversal() {
        let directory = std::env::temp_dir().join(format!(
            "tnpm-unsafe-tar-{}-{}",
            std::process::id(),
            unique()
        ));
        fs::create_dir(&directory).unwrap();
        for link in [false, true] {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(0);
            if link {
                header.set_path("package/escape").unwrap();
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("../../outside").unwrap();
            } else {
                let name = b"package/../../outside";
                header.as_mut_bytes()[..name.len()].copy_from_slice(name);
            }
            header.set_cksum();
            let mut builder = tar::Builder::new(Vec::new());
            builder.append(&header, &[][..]).unwrap();
            let bytes = builder.into_inner().unwrap();
            assert!(unpack(&bytes, &directory).is_err());
            assert!(!directory.join("escape").exists());
        }
        fs::remove_dir(directory).unwrap();
    }
}
