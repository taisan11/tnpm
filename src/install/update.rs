//! Explicit major upgrades; peer validation still runs before any project writes.
use super::{CONCURRENCY, context::Context, resolver::descriptor};
use crate::{json::Value, lockfile, semver, spec};
use anyhow::{Context as _, Result, bail};
use futures_util::{StreamExt, stream};
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;

pub(super) async fn latest(
    ctx: &Context<'_>,
    manifest: &mut Value,
) -> Result<FxHashMap<(String, String), Value>> {
    let mut targets = BTreeSet::new();
    for argument in &ctx.options.arguments {
        let (name, selector) = spec::package_spec(argument)?;
        if selector.is_some() {
            bail!("update --latest accepts package names, not version specifiers: {argument}");
        }
        targets.insert(name);
    }
    let mut requests = Vec::new();
    let mut matched = BTreeSet::new();
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        for (name, value) in manifest.get(field).map(Value::members).unwrap_or_default() {
            if !targets.is_empty() && !targets.contains(name.as_str()) {
                continue;
            }
            if let Some(range) = value.string() {
                if !lockfile::valid_name(name) {
                    bail!("invalid dependency name: {name}");
                }
                spec::registry_dependency(name, range)?;
                matched.insert(name.as_str());
                requests.push((field, name.clone(), range.to_owned()));
            }
        }
    }
    if let Some(name) = targets.difference(&matched).next() {
        bail!("update --latest: no declared dependency named {name}");
    }
    let responses: Vec<_> =
        stream::iter(requests.into_iter().map(|(field, name, range)| async move {
            let (target, _) = spec::registry_dependency(&name, &range)?;
            let data = ctx.metadata(target.as_ref(), "latest").await?;
            let version = data
                .str("version")
                .context("registry version has no version")?;
            let saved = saved_spec(ctx, &name, &range, version)?;
            Ok::<_, anyhow::Error>((field, name, saved, descriptor(&data)?))
        }))
        .buffered(CONCURRENCY)
        .collect()
        .await;
    let responses = responses.into_iter().collect::<Result<Vec<_>>>()?;
    let mut memo = FxHashMap::default();
    for (field, name, saved, entry) in responses {
        manifest
            .get_mut(field)
            .context("dependency field disappeared")?
            .set(&name, Value::from(saved.as_str()));
        memo.insert((name, saved), entry);
    }
    Ok(memo)
}

fn saved_spec(ctx: &Context<'_>, name: &str, original: &str, version: &str) -> Result<String> {
    let (target, range) = spec::registry_dependency(name, original)?;
    let prefix = if ctx.options.save_exact || ctx.config.enabled("save-exact") {
        ""
    } else if range.starts_with('^') {
        "^"
    } else if range.starts_with('~') {
        "~"
    } else if semver::Version::parse(range).is_some() {
        ""
    } else {
        ctx.config.get("save-prefix").unwrap_or("^")
    };
    let saved = format!("{prefix}{version}");
    Ok(if original.starts_with("jsr:") {
        let jsr = crate::jsr::Specifier::parse(original)?;
        format!("jsr:{}@{saved}", jsr.name)
    } else if original.starts_with("npm:") {
        format!("npm:{target}@{saved}")
    } else {
        saved
    })
}
