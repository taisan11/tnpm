//! Install-related command orchestration; resolution, storage, and tree mutation are separate.
use crate::{
    cli::Options,
    config::Config,
    fs::write_json,
    json::Value,
    lockfile::{self, Lockfile},
    manifest, scripts, semver, spec,
    util::style::table,
};
use anyhow::{Context as _, Result, bail};
use futures_util::{StreamExt, stream};
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

mod audit;
mod bundles;
mod context;
mod isolated;
mod overrides;
mod platform;
mod report;
mod resolver;
mod sources;
mod store;
mod tree;
mod trust;
mod update;

use context::Context;
use resolver::{descriptor, resolve};
use tree::materialize;

const CONCURRENCY: usize = 16;

/// Runner imports use the same verified installer, isolated from project manifests.
pub(crate) async fn runtime_package(
    project: &Path,
    name: &str,
    selector: &str,
    options: &Options,
    config: &Config,
) -> Result<std::path::PathBuf> {
    let (target, _) = spec::registry_dependency(name, selector)?;
    let registry = config.registry_url(target.as_ref(), options.registry.as_deref())?;
    let key = format!("{name}\n{selector}\n{registry}");
    let root = dirs::home_dir()
        .context("home directory not found")?
        .join(".tnpm/exec")
        .join(format!("runner-{:x}", Sha256::digest(key.as_bytes())));
    fs::create_dir_all(&root)?;
    let mut isolated = options.clone();
    isolated.command = "runner-import".into();
    isolated.json = true;
    isolated.ignore_scripts = true;
    isolated.arguments.clear();
    isolated.passthrough.clear();
    isolated.workspace.clear();
    isolated.workspaces = false;
    let ctx = Context::new(&isolated, config);
    *ctx.source_root.borrow_mut() = fs::canonicalize(&root)?;
    let mut manifest = Value::object();
    manifest.set("name", Value::from("tnpm-runner-import"));
    let mut dependencies = Value::object();
    dependencies.set(name, Value::from(selector));
    manifest.set("dependencies", dependencies);
    let old = if options.no_cache {
        None
    } else {
        Lockfile::read(&root)?
    };
    let lock = resolve(
        &ctx,
        &manifest,
        old.as_ref(),
        options.no_cache,
        FxHashMap::default(),
    )
    .await?;
    lock.write(&root, &manifest)?;
    materialize(&ctx, &root, &manifest, &lock, false)
        .await
        .with_context(|| format!("could not prepare runtime import in {}", project.display()))?;
    Ok(root)
}

pub(crate) fn workspace_manifest(root: &Path, data: &Value) -> Result<Value> {
    sources::effective(root, data).map(|(data, _)| data)
}

pub(crate) fn listing_overrides(
    root: &Value,
    locked: Option<&Value>,
    data: &mut Value,
) -> Result<()> {
    if let Some(scopes) = locked.and_then(|e| e.get("tnpmOverrideRules")) {
        return overrides::apply_scoped(root, scopes.clone(), data);
    }
    let name = data.str("name").unwrap_or("").to_owned();
    overrides::apply(root, None, &name, data)
}

fn flag(data: &Value, key: &str) -> bool {
    data.get(key) == Some(&Value::Bool(true))
}

fn summary(options: &Options, action: &str, count: usize) {
    if options.command == "runner-import" {
        return;
    }
    crate::util::style::progress::finish();
    if matches!(options.command.as_str(), "exec" | "create") {
        eprintln!(
            "{}",
            crate::util::style::stderr(
                &format!("{action} {count} packages"),
                crate::util::style::Tone::Success
            )
        );
    } else if options.json {
        let mut value = Value::object();
        value.set(action, Value::Number(count.to_string()));
        println!("{}", value.pretty());
    } else {
        println!(
            "{}",
            crate::util::style::stdout(
                &format!("{action} {count} packages"),
                crate::util::style::Tone::Success
            )
        );
    }
}

pub async fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    if (options.offline || config.enabled("offline")) && options.no_cache {
        bail!("--offline and --no-cache cannot be combined");
    }
    let ctx = Context::new(options, config);
    *ctx.source_root.borrow_mut() = fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    if let Some(data) = manifest::read(root)? {
        *ctx.workspaces.borrow_mut() = crate::workspaces::discover(root, &data)?;
    }
    match options.command.as_str() {
        "exec" | "create" => return exec(&ctx, root).await,
        "outdated" => return outdated(&ctx, root).await,
        "prune" => return tree::prune(&ctx, root),
        "rebuild" => return tree::rebuild(&ctx, root),
        "audit" => return audit::run(&ctx, root).await,
        _ => {}
    }
    if !options.passthrough.is_empty() {
        bail!("unexpected arguments after --");
    }
    if options.command == "ci" && (!options.arguments.is_empty() || options.lock_only) {
        bail!("ci accepts neither packages nor --package-lock-only");
    }
    let mut manifest = manifest::read(root)?.unwrap_or_else(Value::object);
    trust::Trust::new(&manifest, options)?;
    overrides::validate(&manifest)?;
    if options.command == "ci" && !root.join("package.json").is_file() {
        bail!("ci requires package.json");
    }
    let old = Lockfile::read(root)?;
    let mut memo = FxHashMap::default();
    if options.command == "ci" {
        let (manifest, _) = sources::effective(root, &manifest)?;
        let lock = old.context("ci requires tnpm-lock.json")?;
        lock.validate_with(&manifest, ctx.peer_policy())?;
        for (path, entry) in &lock.packages {
            if entry.get("tnpmBundlePath").is_some()
                || !entry
                    .str("resolved")
                    .is_some_and(|s| s.starts_with("file:"))
            {
                continue;
            }
            let name = entry.str("name").context("source lock entry has no name")?;
            let spec = entry
                .str("tnpmSpec")
                .context("source lock entry has no spec")?;
            let data = match ctx.metadata(name, spec).await {
                Ok(data) => data,
                Err(_) if flag(entry, "optional") => continue,
                Err(error) => return Err(error),
            };
            let mut current = descriptor(&data)?;
            if let Some(scopes) = entry.get("tnpmOverrideRules") {
                overrides::apply_scoped(&manifest, scopes.clone(), &mut current)?;
            } else {
                overrides::apply(&manifest, None, name, &mut current)?;
            }
            for field in lockfile::DEP_FIELDS.iter().copied().chain([
                "name",
                "version",
                "peerDependenciesMeta",
                "integrity",
                "engines",
                "os",
                "cpu",
                "libc",
                "bin",
                "gypfile",
                "hasInstallScript",
            ]) {
                if entry.get(field) != current.get(field) {
                    bail!(
                        "source package.json and lockfile disagree on {path} {field}; run install first"
                    );
                }
            }
        }
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
            tree::snapshot(&ctx, root)?
        })
    } else {
        None
    };
    let selected = if options.workspaces || !options.workspace.is_empty() {
        crate::workspaces::selected(&ctx.workspaces.borrow(), &options.workspace)?
            .iter()
            .map(|w| root.join(&w.relative))
            .collect::<Vec<_>>()
    } else {
        vec![root.to_owned()]
    };
    let mut edits = selected
        .into_iter()
        .map(|target| {
            let data = manifest::read(&target)?.unwrap_or_else(Value::object);
            Ok((target, data))
        })
        .collect::<Result<Vec<_>>>()?;
    for (target, manifest) in &mut edits {
        if options.command == "update" && options.latest {
            memo.extend(update::latest(&ctx, manifest).await?);
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
                let (owned_name, owned_selector) = install_spec(&ctx, spec).await?;
                let name = owned_name.as_str();
                let selector = owned_selector.as_deref();
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
                let spec = if let Some(source) =
                    selector.as_deref().filter(|s| sources::is_source(s))
                {
                    if let Some(path) = source.strip_prefix("file:") {
                        let path = fs::canonicalize(ctx.source_root.borrow().join(path))?;
                        format!(
                            "file:{}",
                            sources::relative(&fs::canonicalize(&*target)?, &path)
                        )
                    } else {
                        source.to_owned()
                    }
                } else if let Some(jsr) = selector.as_deref().filter(|s| s.starts_with("jsr:")) {
                    let parsed = crate::jsr::Specifier::parse(jsr)?;
                    let range = if options.save_exact {
                        version.to_owned()
                    } else if parsed.range == "latest" {
                        format!("^{version}")
                    } else {
                        parsed.range.to_owned()
                    };
                    format!("jsr:{}@{range}", parsed.name)
                } else if let Some(alias) = selector.as_deref().filter(|s| s.starts_with("npm:")) {
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
                if !sources::is_source(&spec) {
                    memo.insert((name, spec), descriptor(&data)?);
                }
            }
        }
    }
    for (target, data) in &edits {
        if target == root {
            manifest = data.clone();
        } else if let Some(workspace) = ctx
            .workspaces
            .borrow_mut()
            .iter_mut()
            .find(|w| root.join(&w.relative) == *target)
        {
            workspace.manifest = data.clone();
        }
    }
    let (effective, _) = sources::effective(root, &manifest)?;
    let lock = resolve(
        &ctx,
        &effective,
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
            for (target, data) in &edits {
                write_json(&target.join("package.json"), data)?;
            }
        }
        lock.write(root, &effective)?;
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
    materialize(&ctx, root, &effective, &lock, false).await?;
    if let Some(before) = &before_update {
        ctx.progress.event(
            "place",
            "verifying installed versions for the update report",
        );
        report::print(options, before, &tree::snapshot(&ctx, root)?);
    }
    Ok(())
}

async fn install_spec(ctx: &Context<'_>, argument: &str) -> Result<(String, Option<String>)> {
    if argument.starts_with("jsr:") {
        let parsed = crate::jsr::Specifier::parse(argument)?;
        anyhow::ensure!(
            parsed.subpath.is_empty(),
            "JSR dependencies cannot select an export subpath"
        );
        return Ok((parsed.name.to_owned(), Some(argument.to_owned())));
    }
    let selector = if argument.starts_with("./")
        || argument.starts_with("../")
        || Path::new(argument).is_absolute()
    {
        Some(format!("file:{argument}"))
    } else if sources::is_source(argument) {
        Some(argument.to_owned())
    } else {
        None
    };
    if let Some(selector) = selector {
        let data = ctx.metadata("tnpm-source", &selector).await?;
        return Ok((
            data.str("name")
                .context("source requires a name")?
                .to_owned(),
            Some(selector),
        ));
    }
    let (name, selector) = spec::package_spec(argument)?;
    Ok((name.to_owned(), selector.map(str::to_owned)))
}

async fn outdated(ctx: &Context<'_>, root: &Path) -> Result<()> {
    let manifest = manifest::read(root)?.context("package.json not found")?;
    let deps: Vec<_> = lockfile::dependencies(&manifest, true)
        .into_iter()
        .filter(|d| !sources::is_source(&d.spec))
        .filter(|d| {
            !ctx.workspaces.borrow().iter().any(|w| {
                w.name == d.name
                    && semver::satisfies(w.manifest.str("version").unwrap_or("0.0.0"), &d.spec)
                        == Some(true)
            })
        })
        .filter(|d| ctx.options.arguments.is_empty() || ctx.options.arguments.contains(&d.name))
        .collect();
    let rows: Vec<_> = stream::iter(deps.into_iter().map(|dep| async move {
        let (target, _) = spec::dependency_target(&dep.name, &dep.spec)?;
        let (wanted, latest) = futures_util::future::try_join(
            ctx.metadata(&dep.name, &dep.spec),
            ctx.metadata(target.as_ref(), "latest"),
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
                crate::util::style::stdout(
                    "All dependencies are up to date.",
                    crate::util::style::Tone::Success,
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
    if let Some(project) = manifest::read(root)?
        && let Some(trusted) = project.get("trustedDependencies")
    {
        manifest.set("trustedDependencies", trusted.clone());
    }
    trust::Trust::new(&manifest, ctx.options)?;
    let mut deps = Value::object();
    for spec in &requested {
        let jsr;
        let (name, selector) = if spec.starts_with("jsr:") {
            jsr = crate::jsr::Specifier::parse(spec)?;
            anyhow::ensure!(
                jsr.subpath.is_empty(),
                "exec does not accept JSR export subpaths"
            );
            (jsr.name, Some(spec.as_str()))
        } else {
            spec::package_spec(spec)?
        };
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
        let name = if requested[0].starts_with("jsr:") {
            crate::jsr::Specifier::parse(&requested[0])?.name
        } else {
            spec::package_spec(&requested[0])?.0
        };
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
