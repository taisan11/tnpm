//! Content-addressed, file-level hardlink store for isolated installations.
//! Unlike a reflink/copy this strictly requires a single filesystem/volume.
use crate::json::Value;
use anyhow::{Context as _, Result, bail, ensure};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

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
        Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
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
            #[cfg(unix)]
            if metadata.permissions().mode() & 0o222 != 0 {
                return Ok(false);
            }
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
    let exec = executable(source)?;
    let digest = checksum(source)?;
    let object = object_path(store, &digest, exec);
    super::tree::ensure_safe_parents(
        store,
        &format!(
            "files-v1/{}/{}",
            &object.file_name().unwrap().to_string_lossy()[..2],
            object.file_name().unwrap().to_string_lossy()
        ),
    )?;
    if valid_object(&object, &digest, exec)? {
        return Ok(object);
    }
    // Copy and hash in one pass; the object name depends on the content, so the
    // temporary file lives under the store and is published afterwards.
    let temporary_dir = store.join("tmp-v1");
    fs::create_dir_all(&temporary_dir)?;
    #[cfg(unix)]
    fs::set_permissions(&temporary_dir, fs::Permissions::from_mode(0o700))?;
    let temporary = temporary_dir.join(format!("{}-{}", std::process::id(), crate::fs::unique()));
    let mut hasher = Sha256::new();
    let mut input = fs::File::open(source)?;
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let copied = (|| -> Result<()> {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        Ok(())
    })();
    drop(output);
    if let Err(error) = copied {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    #[cfg(unix)]
    fs::set_permissions(
        &temporary,
        fs::Permissions::from_mode(if exec { 0o555 } else { 0o444 }),
    )?;
    ensure!(
        <[u8; 32]>::from(hasher.finalize()) == digest,
        "package file changed while publishing"
    );
    let parent = object
        .parent()
        .context("invalid shared store object path")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    let result: Result<()> = match fs::hard_link(&temporary, &object) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Only objects we did not just write need an integrity check.
            if valid_object(&object, &digest, exec)? {
                Ok(())
            } else {
                // Replace a corrupted object with the copy we just verified.
                fs::rename(&temporary, &object)?;
                Ok(())
            }
        }
        Err(error) => Err(error.into()),
    };
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(object)
}

fn replace_file(path: &Path, store: &Path) -> Result<()> {
    let object = publish_file(path, store)?;
    let temporary = path.with_file_name(format!(
        ".tnpm-link-{}-{}",
        std::process::id(),
        crate::fs::unique()
    ));
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
            bail!(
                "package contains a link or special file: {}",
                path.display()
            );
        }
    }
    Ok(())
}

pub(super) fn link_package(directory: &Path, store: &Path) -> Result<()> {
    link_tree(directory, store, 0)
}

fn stamp(metadata: &fs::Metadata) -> Result<String> {
    let modified = metadata.modified()?.duration_since(std::time::UNIX_EPOCH)?;
    #[cfg(unix)]
    let mode = metadata.permissions().mode();
    #[cfg(not(unix))]
    let mode = u32::from(metadata.permissions().readonly());
    let stamp = format!(
        "{}:{}:{}:{mode}",
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    );
    #[cfg(unix)]
    let stamp = {
        use std::fmt::Write as _;
        use std::os::unix::fs::MetadataExt;
        let mut stamp = stamp;
        write!(&mut stamp, ":{}:{}", metadata.dev(), metadata.ino())?;
        stamp
    };
    Ok(stamp)
}

fn indexed_object(store: &Path, record: &Value) -> Result<PathBuf> {
    let name = record.str("object").context("missing indexed object")?;
    let digest = name.strip_suffix("-x").unwrap_or(name);
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid indexed object"
    );
    let relative = format!("files-v1/{}/{}", &name[..2], name);
    super::tree::ensure_safe_parents(store, &relative)?;
    Ok(store.join(relative))
}

fn checked_object(store: &Path, record: &Value) -> Result<Option<PathBuf>> {
    let object = indexed_object(store, record)?;
    let metadata = match fs::symlink_metadata(&object) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !super::tree::is_link(&metadata),
        "unsafe indexed store object"
    );
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o222 != 0 {
        return Ok(None);
    }
    Ok((record.str("stamp") == Some(stamp(&metadata)?.as_str())).then_some(object))
}

fn unchanged(store: &Path, record: &Value) -> Result<bool> {
    Ok(checked_object(store, record)?.is_some())
}

fn index_path(path: &str) -> Result<&Path> {
    let path = Path::new(path);
    ensure!(
        super::store::safe_relative(path)
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "unsafe package index path"
    );
    ensure!(
        path.components().count() < 128,
        "package index nesting exceeds 128 levels"
    );
    Ok(path)
}

pub(super) fn restore(index: &Value, store: &Path, destination: &Path) -> Result<bool> {
    let files = index.get("files").context("missing package index files")?;
    ensure!(
        matches!(files, Value::Object(_)),
        "invalid package index files"
    );
    ensure!(
        files.get("package.json").is_some(),
        "package index has no manifest"
    );
    let directories = match index.get("directories") {
        Some(Value::Array(directories)) => directories,
        _ => bail!("invalid package index directories"),
    };
    let mut objects = Vec::with_capacity(files.members().len());
    for (path, record) in files.members() {
        index_path(path)?;
        let Some(object) = checked_object(store, record)? else {
            return Ok(false);
        };
        objects.push((path, object));
    }
    for directory in directories {
        index_path(directory.string().context("invalid indexed directory")?)?;
    }
    fs::create_dir(destination)?;
    for directory in directories {
        fs::create_dir_all(destination.join(directory.string().unwrap()))?;
    }
    for (path, object) in objects {
        let destination = destination.join(path);
        fs::hard_link(object, &destination).with_context(|| {
            format!("cannot hardlink {}: the project and store-dir must be on the same filesystem/Windows volume", destination.display())
        })?;
    }
    Ok(true)
}

pub(super) fn index_package(
    directory: &Path,
    store: &Path,
    previous: Option<&Value>,
) -> Result<Value> {
    fn visit(
        root: &Path,
        relative: &Path,
        store: &Path,
        previous: Option<&Value>,
        files: &mut Value,
        directories: &mut Vec<Value>,
        depth: usize,
    ) -> Result<()> {
        ensure!(depth < 128, "package directory nesting exceeds 128 levels");
        for entry in fs::read_dir(root.join(relative))? {
            let entry = entry?;
            let path = relative.join(entry.file_name());
            if relative.as_os_str().is_empty() && entry.file_name() == "node_modules" {
                continue;
            }
            let name = path.to_str().context("non-UTF-8 package path")?;
            index_path(name)?;
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(!super::tree::is_link(&metadata), "package contains a link");
            if metadata.is_dir() {
                directories.push(Value::from(name));
                visit(root, &path, store, previous, files, directories, depth + 1)?;
            } else if metadata.is_file() {
                if let Some(record) = previous
                    .and_then(|v| v.get("files"))
                    .and_then(|v| v.get(name))
                    && unchanged(store, record)?
                {
                    files.set(name, record.clone());
                    continue;
                }
                let object = publish_file(&entry.path(), store)?;
                let mut record = Value::object();
                record.set(
                    "object",
                    Value::from(object.file_name().unwrap().to_str().unwrap()),
                );
                record.set(
                    "stamp",
                    Value::from(stamp(&fs::metadata(&object)?)?.as_str()),
                );
                files.set(name, record);
            } else {
                bail!("package contains a special file");
            }
        }
        Ok(())
    }
    let mut files = Value::object();
    let mut directories = Vec::new();
    visit(
        directory,
        Path::new(""),
        store,
        previous,
        &mut files,
        &mut directories,
        0,
    )?;
    let mut index = Value::object();
    index.set("files", files);
    index.set("directories", Value::Array(directories));
    Ok(index)
}
