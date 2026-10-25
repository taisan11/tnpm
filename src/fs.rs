//! Atomic JSON replacement and process-local temporary-file identifiers.
use crate::json::Value;
use anyhow::{Context, Result};
use std::{fs, io::Write, path::Path};

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("JSON path has no parent")?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".tnpm-json-{}-{}", std::process::id(), unique()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = (|| -> Result<()> {
        file.write_all(format!("{}\n", value.pretty()).as_bytes())?;
        drop(file);
        fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.with_context(|| format!("could not write {}", path.display()))
}

pub fn unique() -> u64 {
    static ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
