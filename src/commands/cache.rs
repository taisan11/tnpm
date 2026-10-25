//! Global tnpm cache inspection and removal. Never follow cache symlinks.
use crate::{cli::Options, json::Value, table};
use anyhow::{Context, Result, bail};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Default, PartialEq, Eq)]
struct Usage {
    bytes: u64,
    disk_bytes: u64,
    files: u64,
    symlinks: u64,
}

impl Usage {
    fn add(&mut self, other: &Self) -> Result<()> {
        let add = |a: u64, b: u64| a.checked_add(b).context("cache usage exceeds u64");
        self.bytes = add(self.bytes, other.bytes)?;
        self.disk_bytes = add(self.disk_bytes, other.disk_bytes)?;
        self.files = add(self.files, other.files)?;
        self.symlinks = add(self.symlinks, other.symlinks)?;
        Ok(())
    }

    fn metadata(meta: &fs::Metadata) -> Result<Self> {
        #[cfg(unix)]
        let disk_bytes = {
            use std::os::unix::fs::MetadataExt;
            meta.blocks()
                .checked_mul(512)
                .context("cache disk usage exceeds u64")?
        };
        #[cfg(not(unix))]
        let disk_bytes = if meta.is_file() { meta.len() } else { 0 };
        Ok(Self {
            bytes: if meta.is_file() { meta.len() } else { 0 },
            disk_bytes,
            files: u64::from(meta.is_file()),
            symlinks: u64::from(meta.file_type().is_symlink()),
        })
    }

    fn value(&self) -> Value {
        let mut value = Value::object();
        for (key, number) in [
            ("bytes", self.bytes),
            ("diskBytes", self.disk_bytes),
            ("files", self.files),
            ("symlinks", self.symlinks),
        ] {
            value.set(key, Value::Number(number.to_string()));
        }
        value
    }

    fn row(&self, label: String) -> [String; 4] {
        [
            label,
            size(self.disk_bytes),
            size(self.bytes),
            self.files.to_string(),
        ]
    }
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

fn children(path: &Path) -> Result<Vec<PathBuf>> {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let paths = entries
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<Vec<_>>>()
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(paths)
}

fn measure(path: &Path) -> Result<Usage> {
    let mut total = Usage::default();
    let mut pending = vec![path.to_owned()];
    while let Some(path) = pending.pop() {
        let Some(meta) = metadata(&path)? else {
            continue;
        };
        total.add(&Usage::metadata(&meta)?)?;
        if meta.is_dir() {
            pending.extend(children(&path)?);
        }
    }
    Ok(total)
}

struct Entry {
    path: PathBuf,
    usage: Usage,
    entries: Vec<Entry>,
}

impl Entry {
    fn inspect(path: PathBuf) -> Result<Self> {
        let mut usage = Usage::default();
        let mut entries = Vec::new();
        if let Some(meta) = metadata(&path)? {
            usage.add(&Usage::metadata(&meta)?)?;
            if meta.is_dir() {
                let mut children = children(&path)?;
                children.sort();
                for child in children {
                    let child_usage = measure(&child)?;
                    usage.add(&child_usage)?;
                    entries.push(Self {
                        path: child,
                        usage: child_usage,
                        entries: Vec::new(),
                    });
                }
            }
        }
        Ok(Self {
            path,
            usage,
            entries,
        })
    }

    fn value(&self) -> Value {
        let mut value = self.usage.value();
        value.set("path", Value::from(self.path.to_string_lossy().as_ref()));
        value.set(
            "entries",
            Value::Array(self.entries.iter().map(Self::value).collect()),
        );
        value
    }
}

fn size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB", "PiB", "EiB"] {
        value /= 1024.0;
        unit = next;
        if value < 1024.0 {
            break;
        }
    }
    format!("{value:.2} {unit}")
}

fn paths() -> Result<[PathBuf; 2]> {
    let base = dirs::home_dir()
        .context("home directory not found")?
        .join(".tnpm");
    // Refuse a redirected parent before inspecting or deleting either cache.
    // Cache-root symlinks themselves are counted/unlinked, never traversed.
    if let Some(meta) = metadata(&base)? {
        if meta.file_type().is_symlink() {
            bail!(
                "refusing to access symlinked tnpm directory: {}",
                base.display()
            );
        }
        if !meta.is_dir() {
            bail!("tnpm directory is not a directory: {}", base.display());
        }
    }
    Ok([base.join("cache"), base.join("exec")])
}

pub fn run(options: &Options) -> Result<()> {
    let remove = match options.arguments.as_slice() {
        [] => false,
        [action] if action == "rm" => true,
        _ => bail!("usage: tnpm cache [rm] [--json]"),
    };
    if !options.passthrough.is_empty() {
        bail!("cache does not accept paths or arguments after --");
    }
    let paths = paths()?;
    if remove {
        let mut removed = Vec::new();
        for path in paths {
            let Some(meta) = metadata(&path)? else {
                continue;
            };
            if meta.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
            .with_context(|| format!("could not remove {}", path.display()))?;
            removed.push(Value::from(path.to_string_lossy().as_ref()));
        }
        if options.json {
            let mut value = Value::object();
            value.set("removed", Value::Array(removed));
            println!("{}", value.pretty());
        } else {
            println!(
                "{}",
                crate::style::stdout(
                    if removed.is_empty() {
                        "No tnpm caches to remove."
                    } else {
                        "Removed all tnpm caches."
                    },
                    crate::style::Tone::Success,
                )
            );
        }
        return Ok(());
    }

    let entries = paths
        .into_iter()
        .map(Entry::inspect)
        .collect::<Result<Vec<_>>>()?;
    let mut total = Usage::default();
    for entry in &entries {
        total.add(&entry.usage)?;
    }
    if options.json {
        let mut value = total.value();
        value.set(
            "paths",
            Value::Array(entries.iter().map(Entry::value).collect()),
        );
        println!("{}", value.pretty());
    } else {
        let mut rows = Vec::new();
        for entry in &entries {
            rows.push(entry.usage.row(entry.path.display().to_string()));
            for child in &entry.entries {
                rows.push(child.usage.row(format!(
                    "  {}",
                    child.path.file_name().unwrap().to_string_lossy()
                )));
            }
        }
        rows.push(total.row("Total".into()));
        print!(
            "{}",
            table::format_plain(["Cache", "Disk usage", "Logical size", "Files"], &rows)
        );
        if total.symlinks != 0 {
            println!("Symlinks: {} (targets are not followed).", total.symlinks);
        }
        println!(
            "Shared clone/hardlink blocks may be counted more than once; disk usage is not the space guaranteed to be freed."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units() {
        for (bytes, expected) in [
            (0, "0 B"),
            (1023, "1023 B"),
            (1024, "1.00 KiB"),
            (1024 * 1024, "1.00 MiB"),
        ] {
            assert_eq!(size(bytes), expected);
        }
    }

    #[test]
    fn usage_addition_rejects_overflow() {
        let mut usage = Usage {
            bytes: u64::MAX,
            ..Usage::default()
        };
        assert!(
            usage
                .add(&Usage {
                    bytes: 1,
                    ..Usage::default()
                })
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn disk_usage_uses_allocated_blocks_instead_of_sparse_file_length() {
        use std::os::unix::fs::MetadataExt;
        let path = std::env::temp_dir().join(format!(
            "tnpm-cache-size-{}-{}",
            std::process::id(),
            crate::fs::unique()
        ));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(16 * 1024 * 1024).unwrap();
        let meta = file.metadata().unwrap();
        let usage = measure(&path).unwrap();
        assert_eq!(usage.bytes, 16 * 1024 * 1024);
        assert_eq!(usage.disk_bytes, meta.blocks() * 512);
        assert_eq!(usage.files, 1);
        drop(file);
        fs::remove_file(path).unwrap();
    }
}
