//! Install-related command orchestration; resolution, storage, and tree mutation are separate.
use crate::{
    cli::Options,
    config::Config,
    fs::write_json,
    json::Value,
    lockfile::{self, Lockfile},
    manifest, scripts, semver, spec, table,
};
use anyhow::{Context as _, Result, bail};
use futures_util::{StreamExt, stream};
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

mod context;
mod report;
mod resolver;
mod store;
mod tree;
mod update;

use context::Context;
use resolver::{descriptor, resolve};
use tree::materialize;

const CONCURRENCY: usize = 16;

fn flag(data: &Value, key: &str) -> bool {
    data.get(key) == Some(&Value::Bool(true))
}

fn summary(options: &Options, action: &str, count: usize) {
    crate::progress::finish();
    if matches!(options.command.as_str(), "exec" | "create") {
        eprintln!(
            "{}",
            crate::style::stderr(
                &format!("{action} {count} packages"),
                crate::style::Tone::Success
            )
        );
    } else if options.json {
        let mut value = Value::object();
        value.set(action, Value::Number(count.to_string()));
        println!("{}", value.pretty());
    } else {
        println!(
            "{}",
            crate::style::stdout(
                &format!("{action} {count} packages"),
                crate::style::Tone::Success
            )
        );
    }
}

pub async fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    if (options.offline || config.enabled("offline")) && options.no_cache {
        bail!("--offline and --no-cache cannot be combined");
    }
    let ctx = Context::new(options, config);
    match options.command.as_str() {
        "exec" | "create" => return exec(&ctx, root).await,
        "outdated" => return outdated(&ctx, root).await,
        "prune" => return tree::prune(&ctx, root),
        _ => {}
    }
    if !options.passthrough.is_empty() {
        bail!("unexpected arguments after --");
    }
    if options.command == "ci" && (!options.arguments.is_empty() || options.lock_only) {
        bail!("ci accepts neither packages nor --package-lock-only");
    }
    let mut manifest = manifest::read(root)?.unwrap_or_else(Value::object);
    if options.command == "ci" && !root.join("package.json").is_file() {
        bail!("ci requires package.json");
    }
    let old = Lockfile::read(root)?;
    let mut memo = FxHashMap::default();
    if options.command == "ci" {
        let lock = old.context("ci requires package-lock.json or tnpm-lock.json")?;
        lock.validate_with(&manifest, ctx.peer_policy())?;
        materialize(&ctx, root, &manifest, &lock, true).await?;
        if !options.no_save && !root.join("tnpm-lock.json").try_exists()? {
            lock.write(root, &manifest)?;
        }
        return Ok(());
    }
    let before_update = if options.command == "update" {
        ctx.progress
            .event("prepare", "recording package versions before update");
        Some(if options.lock_only {
            old.clone().unwrap_or_default()
        } else {
            tree::snapshot(root)?
        })
    } else {
        None
    };
    if options.command == "update" && options.latest {
        memo = update::latest(&ctx, &mut manifest).await?;
    } else if options.command == "uninstall" {
        if options.arguments.is_empty() {
            bail!("uninstall requires a package name");
        }
        for name in &options.arguments {
            if !lockfile::valid_name(name) {
                bail!("invalid package name: {name}");
            }
            for field in lockfile::DEP_FIELDS {
                if let Some(map) = manifest.get_mut(field) {
                    map.remove(name);
                }
            }
        }
    } else if options.command == "install" && !options.arguments.is_empty() {
        let values: Vec<_> = stream::iter(options.arguments.iter().map(|spec| async {
            let (name, selector) = spec::package_spec(spec)?;
            let data = ctx
                .metadata(
                    name,
                    selector.unwrap_or(config.get("tag").unwrap_or("latest")),
                )
                .await?;
            Ok::<_, anyhow::Error>((name.to_owned(), selector.map(str::to_owned), data))
        }))
        .buffered(CONCURRENCY)
        .collect()
        .await;
        for result in values {
            let (name, selector, data) = result?;
            let version = data
                .str("version")
                .context("registry version has no version")?;
            let spec = if let Some(alias) = selector.as_deref().filter(|s| s.starts_with("npm:")) {
                let (target, _) = spec::dependency_target(&name, alias)?;
                if options.save_exact {
                    format!("npm:{target}@{version}")
                } else {
                    alias.to_owned()
                }
            } else if options.save_exact {
                version.to_owned()
            } else {
                selector
                    .filter(|s| semver::satisfies(version, s).is_some())
                    .unwrap_or_else(|| format!("^{version}"))
            };
            let field = if options.save_optional {
                "optionalDependencies"
            } else if options.save_dev {
                "devDependencies"
            } else {
                ["dependencies", "devDependencies", "optionalDependencies"]
                    .into_iter()
                    .find(|f| manifest.get(f).and_then(|v| v.get(&name)).is_some())
                    .unwrap_or("dependencies")
            };
            for other in ["dependencies", "devDependencies", "optionalDependencies"] {
                if other != field
                    && let Some(map) = manifest.get_mut(other)
                {
                    map.remove(&name);
                }
            }
            if manifest.get(field).is_none() {
                manifest.set(field, Value::object());
            }
            manifest
                .get_mut(field)
                .context("invalid dependency map")?
                .set(&name, Value::from(spec.as_str()));
            memo.insert((name, spec), descriptor(&data)?);
        }
    }
    let lock = resolve(
        &ctx,
        &manifest,
        old.as_ref(),
        options.command == "update"
            || (options.command == "install" && !options.arguments.is_empty()),
        memo,
    )
    .await?;
    if !options.no_save {
        if options.command == "uninstall"
            || (options.command == "install" && !options.arguments.is_empty())
            || (options.command == "update" && options.latest)
        {
            write_json(&root.join("package.json"), &manifest)?;
        }
        lock.write(root, &manifest)?;
    }
    if options.lock_only {
        ctx.warn_peers(&lock);
        if let Some(before) = &before_update {
            report::print(options, before, &lock);
        } else {
            summary(options, "locked", lock.packages.len().saturating_sub(1));
        }
        return Ok(());
    }
    materialize(&ctx, root, &manifest, &lock, false).await?;
    if let Some(before) = &before_update {
        ctx.progress.event(
            "place",
            "verifying installed versions for the update report",
        );
        report::print(options, before, &tree::snapshot(root)?);
    }
    Ok(())
}

async fn outdated(ctx: &Context<'_>, root: &Path) -> Result<()> {
    let manifest = manifest::read(root)?.context("package.json not found")?;
    let deps: Vec<_> = lockfile::dependencies(&manifest, true)
        .into_iter()
        .filter(|d| ctx.options.arguments.is_empty() || ctx.options.arguments.contains(&d.name))
        .collect();
    let rows: Vec<_> = stream::iter(deps.into_iter().map(|dep| async move {
        let (target, _) = spec::dependency_target(&dep.name, &dep.spec)?;
        let (wanted, latest) = futures_util::future::try_join(
            ctx.metadata(&dep.name, &dep.spec),
            ctx.metadata(target, "latest"),
        )
        .await?;
        let current = manifest::read(&root.join("node_modules").join(&dep.name))?
            .and_then(|m| m.str("version").map(str::to_owned));
        let wanted = wanted.str("version").unwrap_or("").to_owned();
        let latest = latest.str("version").unwrap_or("").to_owned();
        Ok::<_, anyhow::Error>((dep.name, current, wanted, latest))
    }))
    .buffered(CONCURRENCY)
    .collect()
    .await;
    // Finish all lookups before printing, so an error cannot leave a partial table/JSON.
    let rows = rows.into_iter().collect::<Result<Vec<_>>>()?;
    let mut output = Value::object();
    let mut table_rows = Vec::new();
    for row in rows {
        let (name, current, wanted, latest) = row;
        if current.as_deref() == Some(&latest) {
            continue;
        }
        let mut entry = Value::object();
        if let Some(current) = &current {
            entry.set("current", Value::from(current.as_str()));
        }
        entry.set("wanted", Value::from(wanted.as_str()));
        entry.set("latest", Value::from(latest.as_str()));
        entry.set(
            "location",
            Value::from(
                root.join("node_modules")
                    .join(&name)
                    .to_string_lossy()
                    .as_ref(),
            ),
        );
        if !ctx.options.json {
            table_rows.push([
                name.clone(),
                current.as_deref().unwrap_or("MISSING").to_owned(),
                wanted,
                latest,
            ]);
        }
        output.set(&name, entry);
    }
    if ctx.options.json {
        println!("{}", output.pretty());
    } else if !table_rows.is_empty() {
        print!(
            "{}",
            if ctx.options.plain {
                table::format_plain(["Package", "Current", "Wanted", "Latest"], &table_rows)
            } else {
                table::format_bordered(["Package", "Current", "Wanted", "Latest"], &table_rows)
            }
        );
    } else {
        println!(
            "{}",
            if ctx.options.plain {
                "All dependencies are up to date.".to_owned()
            } else {
                crate::style::stdout(
                    "All dependencies are up to date.",
                    crate::style::Tone::Success,
                )
            }
        );
    }
    if !output.members().is_empty() {
        bail!("outdated dependencies found");
    }
    Ok(())
}

async fn exec(ctx: &Context<'_>, root: &Path) -> Result<()> {
    let initializer = ctx.options.command == "create";
    let mut args = ctx.options.arguments.clone();
    args.extend(ctx.options.passthrough.clone());
    let mut requested = ctx.options.exec_packages.clone();
    if initializer {
        let spec = args
            .first()
            .context("create requires an initializer")?
            .clone();
        let (name, selector) = spec::package_spec(&spec)?;
        let name = if let Some((scope, name)) = name.rsplit_once('/') {
            format!("{scope}/create-{name}")
        } else if name.starts_with('@') {
            format!("{name}/create")
        } else {
            format!("create-{name}")
        };
        args[0] = if let Some(selector) = selector {
            format!("{name}@{selector}")
        } else {
            name
        };
    }
    if let Some(call) = &ctx.options.call
        && requested.is_empty()
    {
        return scripts::command(
            root,
            &manifest::read(root)?.unwrap_or_else(Value::object),
            "exec",
            call,
            &args,
            None,
        );
    }
    if requested.is_empty() {
        let spec = args
            .first()
            .context("exec requires a command or --package/--call")?;
        if !initializer && let Some(bin) = scripts::local_bin(root, spec) {
            return scripts::command(
                root,
                &manifest::read(root)?.unwrap_or_else(Value::object),
                "exec",
                &scripts::quote(&bin.to_string_lossy()),
                &args[1..],
                None,
            );
        }
        requested.push(spec.clone());
    }
    let key = format!(
        "{}\n{}",
        requested.join("\n"),
        ctx.config.get("registry").unwrap_or("")
    );
    let cache_root = dirs::home_dir()
        .context("home directory not found")?
        .join(".tnpm/exec")
        .join(format!("{:x}", Sha256::digest(key.as_bytes())));
    fs::create_dir_all(&cache_root)?;
    let mut manifest = Value::object();
    manifest.set("name", Value::from("tnpm-exec"));
    let mut deps = Value::object();
    for spec in &requested {
        let (name, selector) = spec::package_spec(spec)?;
        deps.set(name, Value::from(selector.unwrap_or("latest")));
    }
    manifest.set("dependencies", deps);
    let old = if ctx.options.no_cache {
        None
    } else {
        Lockfile::read(&cache_root)?
    };
    let lock = resolve(ctx, &manifest, old.as_ref(), false, FxHashMap::default()).await?;
    lock.write(&cache_root, &manifest)?;
    materialize(ctx, &cache_root, &manifest, &lock, false).await?;
    let extra = cache_root.join("node_modules/.bin");
    if let Some(call) = &ctx.options.call {
        return scripts::command(root, &manifest, "exec", call, &args, Some(&extra));
    }
    let binary = if ctx.options.exec_packages.is_empty() || initializer {
        let (name, _) = spec::package_spec(&requested[0])?;
        let data = manifest::read(&cache_root.join("node_modules").join(name))?
            .context("executable package was not installed")?;
        let bins = scripts::bins(&data);
        let short = name.rsplit('/').next().unwrap_or(name);
        bins.iter()
            .find(|(n, _)| n == short)
            .or_else(|| {
                (bins.len() == 1
                    || bins
                        .iter()
                        .map(|(_, target)| target)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == 1)
                    .then(|| bins.first())
                    .flatten()
            })
            .map(|(n, _)| n.clone())
            .context("could not determine executable; use --package PACKAGE -- COMMAND")?
    } else {
        args.first()
            .context("exec requires a command with --package")?
            .clone()
    };
    let forwarded = if args.is_empty() { &[][..] } else { &args[1..] };
    scripts::command(
        root,
        &manifest,
        "exec",
        &scripts::quote(&binary),
        forwarded,
        Some(&extra),
    )
}
