//! Tarball fetching, integrity checks, safe extraction, and package importing.
use super::context::Context;
use crate::{fs::unique, json::Value};
use anyhow::{Context as _, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256, Sha512};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::{collections::HashMap, os::unix::fs::MetadataExt, sync::Mutex};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

#[derive(Default)]
pub(super) struct CloneStrategy {
    #[cfg(target_os = "linux")]
    devices: Mutex<HashMap<(u64, u64), bool>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ImportMethod {
    Auto,
    Hardlink,
    Copy,
    CloneOrCopy,
}

impl ImportMethod {
    pub(super) fn configured(ctx: &Context<'_>) -> Self {
        match ctx.config.get("package-import-method") {
            Some("hardlink") => Self::Hardlink,
            Some("copy") => Self::Copy,
            Some("clone-or-copy") => Self::CloneOrCopy,
            _ => Self::Auto,
        }
    }

    fn clones(self) -> bool {
        matches!(self, Self::Auto | Self::CloneOrCopy)
    }

    fn links(self) -> bool {
        matches!(self, Self::Auto | Self::Hardlink)
    }
}

impl CloneStrategy {
    fn supported(&self, source: &Path, destination: &Path) -> std::io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            let parent = destination.parent().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "missing destination parent",
                )
            })?;
            let key = (fs::metadata(source)?.dev(), fs::metadata(parent)?.dev());
            let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(supported) = devices.get(&key) {
                return Ok(*supported);
            }
            let input = fs::File::open(source)?;
            let output = tempfile::NamedTempFile::new_in(parent)?;
            let supported = match reflink(&input, output.as_file()) {
                Ok(()) => true,
                Err(error) => !matches!(
                    error.raw_os_error(),
                    Some(libc::EOPNOTSUPP | libc::ENOTTY | libc::EXDEV)
                ),
            };
            devices.insert(key, supported);
            Ok(supported)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (source, destination);
            Ok(true)
        }
    }

    pub(super) fn clone_tree(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        clone_tree(source, destination, self, ImportMethod::CloneOrCopy)
    }

    pub(super) fn import_tree(
        &self,
        source: &Path,
        destination: &Path,
        method: ImportMethod,
    ) -> std::io::Result<()> {
        clone_tree(source, destination, self, method)
    }
}

pub(super) fn verify(bytes: &[u8], integrity: Option<&str>) -> Result<()> {
    let mut verifier = IntegrityVerifier::new(integrity)?;
    verifier.update(bytes);
    verifier.finish()
}

struct IntegrityVerifier<'a> {
    hasher: Option<Box<dyn sha2::digest::DynDigest>>,
    hashes: Vec<&'a str>,
}

impl<'a> IntegrityVerifier<'a> {
    fn new(integrity: Option<&'a str>) -> Result<Self> {
        let Some(integrity) = integrity else {
            return Ok(Self {
                hasher: None,
                hashes: Vec::new(),
            });
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
            let hasher: Box<dyn sha2::digest::DynDigest> = match algorithm {
                "sha512" => Box::new(Sha512::new()),
                "sha256" => Box::new(Sha256::new()),
                _ => Box::new(sha1::Sha1::new()),
            };
            return Ok(Self {
                hasher: Some(hasher),
                hashes,
            });
        }
        bail!("unsupported tarball integrity algorithm")
    }

    fn update(&mut self, bytes: &[u8]) {
        if let Some(hasher) = &mut self.hasher {
            hasher.update(bytes);
        }
    }

    fn finish(self) -> Result<()> {
        if let Some(hasher) = self.hasher {
            let actual = STANDARD.encode(hasher.finalize());
            ensure!(
                self.hashes.contains(&actual.as_str()),
                "EINTEGRITY: tarball checksum mismatch"
            );
        }
        Ok(())
    }
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
        PathBuf::from(ctx.config.get("store-dir")?)
            .join("packages-v2")
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
fn reflink(source: &fs::File, destination: &fs::File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    // Linux FICLONE: clone file data using the destination filesystem's
    // copy-on-write implementation (e.g. Btrfs, XFS, or APFS-compatible FUSE).
    const FICLONE: libc::c_ulong = 0x4004_9409;
    let result = unsafe { libc::ioctl(destination.as_raw_fd(), FICLONE, source.as_raw_fd()) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn clone_file(source: &Path, destination: &Path) -> std::io::Result<bool> {
    let source_file = fs::File::open(source)?;
    let destination_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    if reflink(&source_file, &destination_file).is_ok() {
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
fn validate_cloned_tree(path: &Path, depth: usize) -> std::io::Result<()> {
    if depth > 128 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "package cache nesting exceeds 128 levels",
        ));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            validate_cloned_tree(&entry.path(), depth + 1)?;
        } else if !kind.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "package cache contains a link or special file",
            ));
        }
    }
    Ok(())
}

fn clone_tree(
    source: &Path,
    destination: &Path,
    strategy: &CloneStrategy,
    method: ImportMethod,
) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    if method.clones()
        && fs::symlink_metadata(source).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && clone_file(source, destination)?
    {
        // clonefile also clones symlinks; validate before the package can be placed.
        if let Err(error) = validate_cloned_tree(destination, 0) {
            let _ = fs::remove_dir_all(destination);
            return Err(error);
        }
        return Ok(());
    }
    clone_tree_files(source, destination, 0, None, strategy, method, None)
}

fn clone_tree_files(
    source: &Path,
    destination: &Path,
    depth: usize,
    supported: Option<bool>,
    strategy: &CloneStrategy,
    method: ImportMethod,
    kind: Option<fs::FileType>,
) -> std::io::Result<()> {
    if depth > 128 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "package cache nesting exceeds 128 levels",
        ));
    }
    let metadata = match kind {
        Some(kind) if !kind.is_dir() => None,
        _ => Some(fs::symlink_metadata(source)?),
    };
    let kind = metadata.as_ref().map(|m| m.file_type()).or(kind).unwrap();
    if kind.is_dir() && !super::tree::is_link(metadata.as_ref().unwrap()) {
        fs::create_dir(destination)?;
        let entries = fs::read_dir(source)?
            .map(|entry| {
                let entry = entry?;
                let kind = entry.file_type()?;
                Ok((entry, kind))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let mut supported = supported;
        if method.clones()
            && supported.is_none()
            && let Some((entry, _)) = entries.iter().find(|(_, kind)| kind.is_file())
        {
            supported =
                Some(strategy.supported(&entry.path(), &destination.join(entry.file_name()))?);
        }
        #[cfg(unix)]
        let links = if method.links() && (!method.clones() || supported == Some(false)) {
            Some(DirectoryLinks::open(source, destination)?)
        } else {
            None
        };
        let import = |(entry, kind): &(fs::DirEntry, fs::FileType)| {
            #[cfg(unix)]
            if let Some(links) = &links
                && kind.is_file()
                && !kind.is_symlink()
            {
                return hardlink_or_copy(
                    &entry.path(),
                    &destination.join(entry.file_name()),
                    |_, _| links.link(&entry.file_name()),
                );
            }
            clone_tree_files(
                &entry.path(),
                &destination.join(entry.file_name()),
                depth + 1,
                supported,
                strategy,
                method,
                Some(*kind),
            )
        };
        if method.links() && depth == 0 && entries.len() > 256 {
            std::thread::scope(|scope| {
                let workers: Vec<_> = entries
                    .chunks(entries.len().div_ceil(4))
                    .map(|chunk| {
                        let import = &import;
                        scope.spawn(move || chunk.iter().try_for_each(import))
                    })
                    .collect();
                for worker in workers {
                    worker
                        .join()
                        .map_err(|_| std::io::Error::other("package import worker panicked"))??;
                }
                Ok::<_, std::io::Error>(())
            })?;
        } else {
            entries.iter().try_for_each(import)?;
        }
        fs::set_permissions(destination, metadata.unwrap().permissions())?;
        return Ok(());
    }
    if !kind.is_file() || kind.is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "package cache contains a link or special file",
        ));
    }
    if method.clones() {
        let supported = match supported {
            Some(supported) => supported,
            None => strategy.supported(source, destination)?,
        };
        if supported && clone_file(source, destination)? {
            return Ok(());
        }
    }
    if method.links() {
        return hardlink_or_copy(source, destination, |source, dest| {
            fs::hard_link(source, dest)
        });
    }
    fs::copy(source, destination)?;
    Ok(())
}

#[cfg(unix)]
struct DirectoryLinks {
    source: fs::File,
    destination: fs::File,
}

#[cfg(unix)]
impl DirectoryLinks {
    fn open(source: &Path, destination: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let open = |path| {
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
                .open(path)
        };
        Ok(Self {
            source: open(source)?,
            destination: open(destination)?,
        })
    }

    fn link(&self, name: &std::ffi::OsStr) -> std::io::Result<()> {
        use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        // Both descriptors stay open and the directory-entry name is NUL-terminated.
        let result = unsafe {
            libc::linkat(
                self.source.as_raw_fd(),
                name.as_ptr(),
                self.destination.as_raw_fd(),
                name.as_ptr(),
                0,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

fn hardlink_or_copy(
    source: &Path,
    destination: &Path,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if link(source, destination).is_err() {
        fs::copy(source, destination)?;
    }
    Ok(())
}

pub(super) fn detach_file(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "cannot detach a link or special file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() <= 1 {
            return Ok(());
        }
    }
    let temporary = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| std::io::Error::other("missing file parent"))?,
    )?;
    fs::copy(path, temporary.path())?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub(super) fn detach_tree(path: &Path, depth: usize) -> std::io::Result<()> {
    if depth > 128 {
        return Err(std::io::Error::other("package nesting exceeds 128 levels"));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if super::tree::is_link(&metadata) {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            detach_tree(&entry?.path(), depth + 1)?;
        }
    } else if metadata.is_file() {
        detach_file(path)?;
    }
    Ok(())
}

pub(super) fn cache_extracted_package(
    source: &Path,
    cache: &Path,
    integrity: Option<&str>,
    strategy: &CloneStrategy,
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
        strategy.clone_tree(source, &package)?;
        publish_package_cache(&temporary, cache, integrity)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

pub(super) fn publish_package_cache(
    temporary: &Path,
    cache: &Path,
    integrity: Option<&str>,
) -> Result<()> {
    #[cfg(unix)]
    fs::set_permissions(
        cache.parent().context("package cache path has no parent")?,
        fs::Permissions::from_mode(0o700),
    )?;
    private_cache_write(
        &temporary.join(".complete"),
        integrity.unwrap_or("").as_bytes(),
    )?;
    match fs::rename(temporary, cache) {
        Ok(()) => Ok(()),
        Err(error) if cache.exists() => {
            let _ = fs::remove_dir_all(temporary);
            if cached_package(cache, integrity).is_some() {
                Ok(())
            } else {
                Err(error.into())
            }
        }
        Err(error) => Err(error.into()),
    }
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
    if let Some(cache) = source_tarball_cache(ctx, entry) {
        let integrity = integrity.map(str::to_owned);
        let result = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
            let bytes = match fs::read(&cache) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(None),
            };
            verify(&bytes, integrity.as_deref())?;
            Ok(Some(bytes))
        })
        .await
        .context("source tarball cache task failed")?;
        if let Some(bytes) = result? {
            return Ok(bytes);
        }
    }
    let response = tarball_response(ctx, &url, name).await?;
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

async fn tarball_response(
    ctx: &Context<'_>,
    url: &reqwest::Url,
    name: &str,
) -> Result<reqwest::Response> {
    let authorization = ctx.config.authorization(url)?;
    if ctx.offline() {
        bail!("ENOTCACHED: tarball {url}");
    }
    let mut request = ctx
        .client(url, true)?
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
    Ok(response)
}

pub(super) async fn prefetch_package(
    ctx: &Context<'_>,
    entry: &Value,
    name: &str,
    cache: PathBuf,
) -> Result<()> {
    let mut profile = crate::profile::Profile::new("archive");
    profile.text("name", name);
    profile.timestamp("started_unix");
    let mut verifier = IntegrityVerifier::new(entry.str("integrity"))?;
    let resolved = entry
        .str("resolved")
        .context("lockfile package has no resolved tarball URL")?;
    let url = reqwest::Url::parse(resolved)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        "only HTTP(S) tarballs without URL credentials are supported"
    );
    let mut response = tarball_response(ctx, &url, name).await?;
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let parent = cache
        .parent()
        .context("package cache path has no parent")?
        .to_owned();
    ctx.progress
        .event("prepare", &format!("extract node_modules/{name}"));
    let extracted = tokio::task::spawn_blocking(move || -> Result<tempfile::TempDir> {
        fs::create_dir_all(&parent)?;
        let temporary = tempfile::Builder::new()
            .prefix(".prefetch-")
            .tempdir_in(parent)?;
        let payload = temporary.path().join("package");
        fs::create_dir(&payload)?;
        unpack_reader(
            ChunkReader {
                receiver,
                current: Cursor::new(Vec::new()),
            },
            &payload,
        )?;
        Ok(temporary)
    });
    let mut sender = Some(sender);
    let downloaded = async {
        while let Some(chunk) = response.chunk().await? {
            verifier.update(&chunk);
            for part in chunk.chunks(64 * 1024) {
                let Some(channel) = &sender else {
                    break;
                };
                if channel.send(part.to_vec()).await.is_err() {
                    sender = None;
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    }
    .await;
    drop(sender);
    profile.mark("download_ms");
    let extracted = extracted
        .await
        .context("streamed package extraction task failed")?;
    downloaded?;
    verifier.finish()?;
    let temporary = extracted?;
    profile.mark("verify_extract_tail_ms");
    let integrity = entry.str("integrity").map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        publish_package_cache(temporary.path(), &cache, integrity.as_deref())
    })
    .await??;
    profile.mark("publish_ms");
    profile.timestamp("finished_unix");
    Ok(())
}

struct ChunkReader {
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl Read for ChunkReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.current.read(output)?;
            if count > 0 {
                return Ok(count);
            }
            let Some(chunk) = self.receiver.blocking_recv() else {
                return Ok(0);
            };
            self.current = Cursor::new(chunk);
        }
    }
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
            .join(format!(
                "{}.tgz",
                const_hex::encode(Sha256::digest(key.as_bytes()))
            )),
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
    unpack_reader(Cursor::new(bytes), dest)
}

fn unpack_reader(mut reader: impl Read, dest: &Path) -> Result<()> {
    let mut prefix = Vec::with_capacity(2);
    reader.by_ref().take(2).read_to_end(&mut prefix)?;
    let gzip = prefix.starts_with(&[0x1f, 0x8b]);
    let reader = Cursor::new(prefix).chain(reader);
    let reader: Box<dyn Read + '_> = if gzip {
        Box::new(flate2::read::GzDecoder::new(reader))
    } else {
        Box::new(reader)
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
    fn streamed_integrity_matches_whole_archive_verification() {
        let bytes = b"package archive";
        for (algorithm, hash) in [
            ("sha512", STANDARD.encode(Sha512::digest(bytes))),
            ("sha256", STANDARD.encode(Sha256::digest(bytes))),
            ("sha1", STANDARD.encode(sha1::Sha1::digest(bytes))),
        ] {
            let integrity = format!("{algorithm}-incorrect {algorithm}-{hash}?option");
            for size in [1, 3, 64] {
                let mut verifier = IntegrityVerifier::new(Some(&integrity)).unwrap();
                for chunk in bytes.chunks(size) {
                    verifier.update(chunk);
                }
                verifier.finish().unwrap();
            }
            let mut verifier = IntegrityVerifier::new(Some(&integrity)).unwrap();
            verifier.update(&bytes[..3]);
            assert!(verifier.finish().is_err());
        }
        IntegrityVerifier::new(None).unwrap().finish().unwrap();
        assert!(IntegrityVerifier::new(Some("md5-value")).is_err());
        let mut verifier = IntegrityVerifier::new(Some("sha512-incorrect")).unwrap();
        verifier.update(b"");
        assert!(verifier.finish().is_err());
    }

    #[test]
    fn chunk_reader_handles_empty_buffers_chunks_and_closed_streams() {
        for chunks in [vec![], vec![b"".to_vec(), b"a".to_vec(), b"bc".to_vec()]] {
            let (sender, receiver) = tokio::sync::mpsc::channel(4);
            let expected = chunks.concat();
            for chunk in chunks {
                sender.blocking_send(chunk).unwrap();
            }
            drop(sender);
            let mut reader = ChunkReader {
                receiver,
                current: Cursor::new(Vec::new()),
            };
            assert_eq!(reader.read(&mut []).unwrap(), 0);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
            assert_eq!(reader.read(&mut [0]).unwrap(), 0);
        }
    }

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

    #[cfg(target_os = "linux")]
    #[test]
    fn unsupported_reflinks_copy_files_without_sharing_inodes() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"original").unwrap();
        fs::set_permissions(source.join("file"), fs::Permissions::from_mode(0o755)).unwrap();
        let strategy = CloneStrategy::default();
        let device = fs::metadata(&source).unwrap().dev();
        strategy
            .devices
            .lock()
            .unwrap()
            .insert((device, device), false);
        for n in 0..2 {
            let destination = root.path().join(format!("destination-{n}"));
            strategy.clone_tree(&source, &destination).unwrap();
            assert_eq!(fs::read(destination.join("file")).unwrap(), b"original");
            assert_eq!(
                fs::metadata(destination.join("file"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o755
            );
            assert_ne!(
                fs::metadata(source.join("file")).unwrap().ino(),
                fs::metadata(destination.join("file")).unwrap().ino()
            );
            fs::write(destination.join("file"), b"changed").unwrap();
        }
        assert_eq!(fs::read(source.join("file")).unwrap(), b"original");
        assert_eq!(strategy.devices.lock().unwrap().len(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reflink_probe_is_shared_across_packages_and_threads() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"contents").unwrap();
        let strategy = CloneStrategy::default();
        std::thread::scope(|scope| {
            for n in 0..4 {
                let strategy = &strategy;
                let source = &source;
                let destination = root.path().join(format!("destination-{n}"));
                scope.spawn(move || {
                    strategy.clone_tree(source, &destination).unwrap();
                    assert_eq!(fs::read(destination.join("file")).unwrap(), b"contents");
                });
            }
        });
        assert_eq!(strategy.devices.lock().unwrap().len(), 1);
        assert!(CloneStrategy::default().devices.lock().unwrap().is_empty());
    }

    #[test]
    fn empty_trees_and_failed_copies_do_not_require_a_probe() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        let strategy = CloneStrategy::default();
        let destination = root.path().join("destination");
        strategy.clone_tree(&source, &destination).unwrap();
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        assert!(strategy.clone_tree(&source, &destination).is_err());
        assert!(
            strategy
                .clone_tree(&root.path().join("missing"), &root.path().join("other"))
                .is_err()
        );
        #[cfg(target_os = "linux")]
        assert!(strategy.devices.lock().unwrap().is_empty());
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

        CloneStrategy::default()
            .import_tree(&source, &destination, ImportMethod::Copy)
            .unwrap();
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

    #[cfg(target_os = "linux")]
    #[test]
    fn auto_links_after_an_unsupported_clone_and_detaches_before_writes() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), "original").unwrap();
        let strategy = CloneStrategy::default();
        let device = fs::metadata(&source).unwrap().dev();
        strategy
            .devices
            .lock()
            .unwrap()
            .insert((device, device), false);
        let dest = root.path().join("destination");
        strategy
            .import_tree(&source, &dest, ImportMethod::Auto)
            .unwrap();
        assert_eq!(
            fs::metadata(source.join("file")).unwrap().ino(),
            fs::metadata(dest.join("file")).unwrap().ino()
        );
        assert!(fs::metadata(dest.join("file")).unwrap().nlink() > 1);
        detach_file(&dest.join("file")).unwrap();
        fs::write(dest.join("file"), "modified").unwrap();
        assert_eq!(fs::read_to_string(source.join("file")).unwrap(), "original");
    }

    #[test]
    fn hardlink_import_copies_across_devices() {
        for kind in [
            std::io::ErrorKind::CrossesDevices,
            std::io::ErrorKind::Unsupported,
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("source");
            let dest = root.path().join("dest");
            fs::write(&source, "original").unwrap();
            hardlink_or_copy(&source, &dest, |_, _| Err(std::io::Error::from(kind))).unwrap();
            assert_eq!(fs::read_to_string(&dest).unwrap(), "original");
            fs::write(&dest, "changed").unwrap();
            assert_eq!(fs::read_to_string(&source).unwrap(), "original");
        }
    }

    #[cfg(unix)]
    #[test]
    fn large_packages_link_in_parallel_and_still_reject_links() {
        use std::os::unix::fs::MetadataExt;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        for n in 0..300 {
            fs::write(source.join(format!("file-{n}")), "content").unwrap();
        }
        let dest = root.path().join("dest");
        let strategy = CloneStrategy::default();
        strategy
            .import_tree(&source, &dest, ImportMethod::Hardlink)
            .unwrap();
        for n in 0..300 {
            assert_eq!(
                fs::metadata(source.join(format!("file-{n}")))
                    .unwrap()
                    .ino(),
                fs::metadata(dest.join(format!("file-{n}"))).unwrap().ino()
            );
        }
        std::os::unix::fs::symlink(root.path(), source.join("escape")).unwrap();
        assert!(
            strategy
                .import_tree(&source, &root.path().join("unsafe"), ImportMethod::Hardlink)
                .is_err()
        );
    }

    #[test]
    fn all_import_methods_handle_empty_and_missing_trees() {
        for method in [
            ImportMethod::Auto,
            ImportMethod::Copy,
            ImportMethod::Hardlink,
            ImportMethod::CloneOrCopy,
        ] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("empty");
            fs::create_dir(&source).unwrap();
            let dest = root.path().join("dest");
            let strategy = CloneStrategy::default();
            strategy.import_tree(&source, &dest, method).unwrap();
            assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
            assert!(
                strategy
                    .import_tree(
                        &root.path().join("missing"),
                        &root.path().join("other"),
                        method
                    )
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_import_rejects_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink(root.path(), source.join("escape")).unwrap();
        assert!(
            CloneStrategy::default()
                .import_tree(&source, &root.path().join("dest"), ImportMethod::Hardlink)
                .is_err()
        );
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
        assert!(
            CloneStrategy::default()
                .clone_tree(&source, &destination)
                .is_err()
        );
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
