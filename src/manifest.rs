//! Reading package manifests without depending on a command implementation.
use crate::json::Value;
use anyhow::{Context, Result, bail};
use std::{fs, io, path::Path};

pub fn read(root: &Path) -> Result<Option<Value>> {
    let path = root.join("package.json");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    let value =
        Value::parse(&text).with_context(|| format!("could not parse {}", path.display()))?;
    if !matches!(value, Value::Object(_)) {
        bail!("{} must contain a JSON object", path.display());
    }
    Ok(Some(value))
}
