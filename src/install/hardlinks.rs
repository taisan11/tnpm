//! Content-addressed, file-level hardlink store for isolated installations.
//! Unlike a reflink/copy this strictly requires a single filesystem/volume.
use anyhow::{Context as _, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn checksum(path: &Path) -> Result<[u8; 32]> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash.finalize().into())
}

fn object_path(store: &Path, digest: &[u8; 32], executable: bool) -> PathBuf {
    let mut name = String::with_capacity(66);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut name, "{byte:02x}").expect("hex string");
    }
    if executable {
        name.push_str("-x");
    }
    store.join("files-v1").join(&name[..2]).join(name)
}

fn executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        return Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn valid_object(path: &Path, digest: &[u8; 32], expected_exec: bool) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "shared hardlink store contains an unsafe entry: {}",
                path.display()
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "shared hardlink store contains a reparse point: {}",
                    path.display()
                );
            }
            Ok(executable(path)? == expected_exec && checksum(path)? == *digest)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn publish_file(source: &Path, store: &Path) -> Result<PathBuf> {
    let digest = checksum(source)?;
    let exec = executable(source)?;
    let object = object_path(store, &digest, exec);
    if valid_object(&object, &digest, exec)? {
        return Ok(object);
    }
    let parent = object.parent().context("invalid shared store object path")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    // Existing corrupted cache names must never be used to serve new installs.
    if fs::symlink_metadata(&object).is_ok() {
        fs::remove_file(&object)?;
    }
    let temporary = parent.join(format!(".{}-{}", std::process::id(), crate::fs::unique()));
    let result = (|| -> Result<()> {
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        #[cfg(unix)]
        fs::set_permissions(
            &temporary,
            fs::Permissions::from_mode(if exec { 0o555 } else { 0o444 }),
        )?;
        // Publish with create-if-absent semantics, preserving other processes'
        // simultaneous objects; on success the inode is immutable on Unix.
        match fs::hard_link(&temporary, &object) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    ensure!(
        valid_object(&object, &digest, exec)?,
        "shared hardlink store object failed integrity verification: {}",
        object.display()
    );
    Ok(object)
}

fn replace_file(path: &Path, store: &Path) -> Result<()> {
    let object = publish_file(path, store)?;
    let temporary = path.with_file_name(format!(".tnpm-link-{}-{}", std::process::id(), crate::fs::unique()));
    fs::hard_link(&object, &temporary).with_context(|| {
        format!(
            "cannot hardlink {} from {}: the project and store-dir must be on the same filesystem/Windows volume",
            path.display(),
            object.display()
        )
    })?;
    let result = (|| -> Result<()> {
        // Ensure hardlink creation succeeded before removing the staged file.
        fs::remove_file(path)?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn link_tree(directory: &Path, store: &Path, depth: usize) -> Result<()> {
    ensure!(depth < 128, "package directory nesting exceeds 128 levels");
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                fs::symlink_metadata(&path)?.file_attributes() & 0x400 == 0,
                "package contains a reparse point: {}",
                path.display()
            );
        }
        if kind.is_dir() && !kind.is_symlink() {
            link_tree(&path, store, depth + 1)?;
        } else if kind.is_file() && !kind.is_symlink() {
            replace_file(&path, store)?;
        } else {
            bail!("package contains a link or special file: {}", path.display());
        }
    }
    Ok(())
}

pub(super) fn link_package(directory: &Path, store: &Path) -> Result<()> {
    link_tree(directory, store, 0)
}
