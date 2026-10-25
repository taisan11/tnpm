use crate::{cli::Options, config::Config, json::Value, manifest, scripts};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    let manifest = manifest::read(root)?.context("package.json not found")?;
    let scripts = manifest
        .get("scripts")
        .cloned()
        .unwrap_or_else(Value::object);
    let Some(event) = options.arguments.first() else {
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
        if options.if_present {
            return Ok(());
        }
        bail!("missing script: {event}");
    };
    let hooks = !options.ignore_scripts && !config.enabled("ignore-scripts");
    if hooks {
        scripts::lifecycle(root, &manifest, &format!("pre{event}"))?;
    }
    let mut args = options.arguments[1..].to_vec();
    args.extend(options.passthrough.clone());
    scripts::command(root, &manifest, event, text, &args, None)?;
    if hooks {
        scripts::lifecycle(root, &manifest, &format!("post{event}"))?;
    }
    Ok(())
}
