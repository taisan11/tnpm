use crate::{cli::Options, config::Config, json::Value, manifest, scripts};
use anyhow::{Context, Result, bail};
use std::{env, path::Path};

pub fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    let manifest = manifest::read(root)?;
    let scripts = manifest
        .as_ref()
        .and_then(|manifest| manifest.get("scripts"))
        .cloned()
        .unwrap_or_else(Value::object);
    let (event, args) = if let Some(event) = options.arguments.first() {
        let mut args = options.arguments[1..].to_vec();
        args.extend(options.passthrough.iter().cloned());
        (Some(event), args)
    } else {
        (
            options.passthrough.first(),
            options.passthrough.iter().skip(1).cloned().collect(),
        )
    };
    let Some(event) = event else {
        manifest.as_ref().context("package.json not found")?;
        if options.json {
            println!("{}", scripts.pretty());
        } else {
            for (name, text) in scripts.members() {
                println!("{name}\n  {}", text.display());
            }
        }
        return Ok(());
    };
    let Some(text) = scripts.str(event) else {
        if is_entry(event) {
            let cwd = if options.prefix.is_some() {
                root.to_owned()
            } else {
                env::current_dir().context("could not determine current directory")?
            };
            let entry = cwd.join(event);
            return crate::runner::run(&cwd, &entry, &args, options, config);
        }
        manifest.as_ref().context("package.json not found")?;
        if options.if_present {
            return Ok(());
        }
        bail!("missing script: {event}");
    };
    let manifest = manifest.as_ref().context("package.json not found")?;
    let shim = crate::toolchain::shim(root, config)?;
    let tool_bin = shim.as_ref().map(|temp| temp.path());
    let hooks = !options.ignore_scripts && !config.enabled("ignore-scripts");
    if hooks {
        scripts::lifecycle_with_bin(root, manifest, &format!("pre{event}"), tool_bin)?;
    }
    scripts::command(root, manifest, event, text, &args, tool_bin)?;
    if hooks {
        scripts::lifecycle_with_bin(root, manifest, &format!("post{event}"), tool_bin)?;
    }
    Ok(())
}

fn is_entry(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|ext| ext.to_str()),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts")
    )
}
