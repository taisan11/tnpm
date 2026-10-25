use crate::{cli::Options, fs::write_json, json::Value, manifest};
use anyhow::Result;
use std::path::Path;

pub fn run(root: &Path, options: &Options) -> Result<()> {
    let mut manifest = manifest::read(root)?.unwrap_or_else(Value::object);
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("package")
        .to_lowercase()
        .replace(' ', "-");
    for (key, value) in [
        ("name", name.as_str()),
        ("version", "1.0.0"),
        ("description", ""),
        ("main", "index.js"),
        ("license", "ISC"),
    ] {
        if manifest.get(key).is_none() {
            manifest.set(key, Value::from(value));
        }
    }
    if manifest.get("scripts").is_none() {
        let mut scripts = Value::object();
        scripts.set(
            "test",
            Value::from("echo \"Error: no test specified\" && exit 1"),
        );
        manifest.set("scripts", scripts);
    }
    write_json(&root.join("package.json"), &manifest)?;
    if options.json {
        println!("{}", manifest.pretty());
    } else {
        println!(
            "{}",
            crate::style::stdout(
                &format!("Wrote {}", root.join("package.json").display()),
                crate::style::Tone::Success
            )
        );
    }
    Ok(())
}
