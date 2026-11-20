//! Atomic JSON replacement and process-local temporary-file identifiers.
use crate::json::Value;
use anyhow::{Context, Result};
use std::{fs, io::Write, path::Path};

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    write_text(path, &format!("{}\n", value.pretty()))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    let parent = path.parent().context("JSON path has no parent")?;
    fs::create_dir_all(parent)?;
    // A crashed process can leave a temporary file whose name a recycled PID
    // would reuse. Keep advancing the suffix instead of failing the write.
    let (temp, mut file) = loop {
        let candidate = parent.join(format!(".tnpm-json-{}-{}", std::process::id(), unique()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let result = (|| -> Result<()> {
        file.write_all(text.as_bytes())?;
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
