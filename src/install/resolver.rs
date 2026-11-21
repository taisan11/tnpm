//! Physical dependency placement and the parallel peer-free resolution path.
use super::{CONCURRENCY, context::Context, flag};
use crate::{
    json::Value,
    lockfile::{self, Dependency, Lockfile},
    spec,
};
use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

mod peers;
mod pipeline;

pub(super) fn descriptor(data: &Value) -> Result<Value> {
    let mut entry = lockfile::copy_fields(
        data,
        &[
            "name",
            "version",
            "dependencies",
            "optionalDependencies",
            "peerDependencies",
            "peerDependenciesMeta",
            "bin",
            "license",
            "engines",
            "os",
            "cpu",
            "funding",
            "libc",
            "gypfile",
            "hasInstallScript",
            "bundleDependencies",
            "bundledDependencies",
            "tnpmSpec",
            "link",
            "tnpmWorkspace",
            "tnpmBundled",
        ],
    );
    if data.get("tnpmWorkspace").is_some()
        && let Some(dev) = data.get("devDependencies")
    {
        entry.set("devDependencies", dev.clone());
    }
    let dist = data
        .get("dist")
        .context("registry version has no dist object")?;
    entry.set(
        "resolved",
        Value::from(
            dist.str("tarball")
                .context("registry version has no tarball")?,
        ),
    );
    if let Some(integrity) = dist.str("integrity") {
        entry.set("integrity", Value::from(integrity));
    } else if let Some(shasum) = dist.str("shasum")
        && shasum.len() == 40
        && shasum.is_ascii()
    {
        let bytes = (0..40)
            .step_by(2)
            .map(|i| u8::from_str_radix(&shasum[i..i + 2], 16))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        entry.set(
            "integrity",
            Value::from(format!("sha1-{}", STANDARD.encode(bytes)).as_str()),
        );
    }
    if ["preinstall", "install", "postinstall"]
        .iter()
        .any(|s| data.get("scripts").and_then(|v| v.str(s)).is_some())
        || data.get("gypfile") == Some(&Value::Bool(true))
    {
        entry.set("hasInstallScript", Value::Bool(true));
    }
    Ok(entry)
}

fn matches(entry: &Value, name: &str, range: &str) -> bool {
    spec::satisfies(name, range, entry) == Some(true)
}

/// Declared non-peer spec for `name`, matching `lockfile::dependencies` precedence
/// (later dependency fields win) without allocating a dependency list.
fn declared_spec<'a>(data: &'a Value, name: &str, root: bool) -> Option<&'a str> {
    let mut spec = None;
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if field == "devDependencies" && !root && data.get("tnpmWorkspace").is_none() {
            continue;
        }
        for (key, value) in data.get(field).map(Value::members).unwrap_or_default() {
            if key == name {
                spec = value.string();
            }
        }
    }
    spec
}

fn index_package(
    lock: &Lockfile,
    dependents: &mut FxHashMap<String, FxHashSet<String>>,
    path: &str,
) {
    let Some(data) = lock.packages.get(path) else {
        return;
    };
    for dep in lockfile::dependencies(data, path.is_empty())
        .into_iter()
        .filter(|dep| !dep.peer)
    {
        dependents
            .entry(dep.name)
            .or_default()
            .insert(path.to_owned());
    }
}

// A hoisted package must not shadow a previously satisfied dependency with an
// incompatible version. Only packages below the candidate's parent can see it.
fn preserves_dependencies(
    lock: &mut Lockfile,
    dependents: &FxHashMap<String, FxHashSet<String>>,
    path: &str,
    entry: &Value,
    name: &str,
) -> bool {
    let scope = lockfile::parent(path);
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}/node_modules/")
    };
    let in_scope = |from: &str| scope.is_empty() || from == scope || from.starts_with(&prefix);
    let mut affected: Vec<(String, String)> = Vec::new();
    if let Some(froms) = dependents.get(name) {
        for from in froms {
            if !in_scope(from) {
                continue;
            }
            let Some(data) = lock.packages.get(from) else {
                continue;
            };
            let Some(spec) = declared_spec(data, name, from.is_empty()) else {
                continue;
            };
            if lock
                .resolve(from, name)
                .is_some_and(|p| matches(&lock.packages[&p], name, spec))
            {
                affected.push((from.clone(), spec.to_owned()));
            }
        }
    }
    lock.packages.insert(path.to_owned(), entry.clone());
    let valid = affected.iter().all(|(from, spec)| {
        lock.resolve(from, name)
            .is_some_and(|p| matches(&lock.packages[&p], name, spec))
    });
    lock.packages.remove(path);
    valid
}

pub(super) async fn resolve(
    ctx: &Context<'_>,
    manifest: &Value,
    old: Option<&Lockfile>,
    update: bool,
    mut memo: FxHashMap<(String, String), Value>,
) -> Result<Lockfile> {
    let mut profile = crate::profile::Profile::new("resolve");
    ctx.progress.event("resolve", "checking dependency graph");
    super::overrides::validate(manifest)?;
    // Workspace and file: sources can change without the root manifest, so they
    // always re-resolve. Git/tarball locks are pinned and may be reused.
    let sources_reusable = manifest.get("workspaces").is_none()
        && old.is_some_and(|lock| {
            lock.packages.values().all(|entry| {
                entry.str("tnpmSpec").is_none_or(|spec| {
                    super::sources::is_git(spec) || super::sources::is_tarball(spec)
                })
            })
        });
    let policy = ctx.peer_policy();
    if !update
        && sources_reusable
        && let Some(old) = old
        && old.validate_root(manifest).is_ok()
    {
        let mut lock = old.clone();
        if let Ok(used) = lock.reuse(manifest, policy) {
            lock.packages
                .retain(|p, _| p.is_empty() || used.contains(p));
            lock.packages
                .insert("".into(), lockfile::root_entry(manifest));
            ctx.progress.event(
                "resolve",
                &format!(
                    "reusing {} locked packages",
                    lock.packages.len().saturating_sub(1)
                ),
            );
            return Ok(lock);
        }
    }
    profile.mark("lock_reuse_ms");
    let mut lock = Lockfile::default();
    lock.packages
        .insert("".into(), lockfile::root_entry(manifest));
    let mut dependents: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();
    index_package(&lock, &mut dependents, "");
    let mut pending: Vec<(String, Dependency)> = lockfile::dependencies(manifest, true)
        .into_iter()
        .filter(|d| !d.peer)
        .map(|d| (String::new(), d))
        .collect();
    let mut pipeline = pipeline::Pipeline::new(
        ctx,
        manifest.get("overrides").is_none() && manifest.get("workspaces").is_none(),
    );
    while !pending.is_empty() {
        if lock.packages.len() > 50_000 {
            bail!("dependency graph exceeds 50,000 packages");
        }
        let mut requests = BTreeSet::new();
        for (from, dep) in &pending {
            if !lockfile::valid_name(&dep.name) {
                bail!("invalid dependency name: {}", dep.name);
            }
            if lock.resolve(from, &dep.name).is_some_and(|p| {
                matches(&lock.packages[&p], &dep.name, &dep.spec)
                    && super::overrides::same_scope(
                        manifest,
                        lock.packages.get(from),
                        &dep.name,
                        &lock.packages[&p],
                    )
            }) {
                continue;
            }
            let key = (dep.name.clone(), dep.spec.clone());
            let refresh = update
                && (ctx.options.arguments.is_empty()
                    || ctx
                        .options
                        .arguments
                        .iter()
                        .any(|s| spec::package_spec(s).is_ok_and(|(n, _)| n == dep.name)));
            let locked = (!refresh && manifest.get("overrides").is_none())
                .then(|| {
                    old.and_then(|l| l.resolve(from, &dep.name).map(|p| l.packages[&p].clone()))
                })
                .flatten()
                .filter(|e| {
                    matches(e, &dep.name, &dep.spec)
                        && !e.str("resolved").is_some_and(|s| s.starts_with("file:"))
                });
            if let Some(entry) = locked {
                memo.entry(key).or_insert(entry);
            } else if !memo.contains_key(&key) {
                requests.insert(key);
            }
        }
        pipeline.resolve(requests, &mut memo).await;
        let mut next = Vec::new();
        for (from, dep) in pending {
            if let Some(path) = lock.resolve(&from, &dep.name)
                && matches(&lock.packages[&path], &dep.name, &dep.spec)
                && super::overrides::same_scope(
                    manifest,
                    lock.packages.get(&from),
                    &dep.name,
                    &lock.packages[&path],
                )
            {
                continue;
            }
            let key = (dep.name.clone(), dep.spec.clone());
            let Some(entry) = memo.get(&key).cloned() else {
                if dep.optional
                    || lock
                        .packages
                        .get(&from)
                        .is_some_and(|v| flag(v, "optional"))
                {
                    crate::util::style::progress::message(format_args!(
                        "tnpm: skipped optional {}: {}",
                        dep.name,
                        pipeline
                            .errors
                            .get(&key)
                            .map(|error| format!("{error:#}"))
                            .unwrap_or_else(|| "resolution failed".into())
                    ));
                    continue;
                }
                return Err(pipeline
                    .errors
                    .remove(&key)
                    .unwrap_or_else(|| anyhow!("resolution failed")))
                .with_context(|| format!("{}@{}", dep.name, dep.spec));
            };
            let mut ancestors = vec![from.as_str()];
            while !ancestors.last().unwrap().is_empty() {
                ancestors.push(lockfile::parent(ancestors.last().unwrap()));
            }
            // Hoist as far as possible without crossing a conflicting ancestor.
            let mut location = None;
            for ancestor in &ancestors {
                let candidate = lockfile::child(ancestor, &dep.name);
                if let Some(existing) = lock.packages.get(&candidate) {
                    if matches(existing, &dep.name, &dep.spec)
                        && super::overrides::same_scope(
                            manifest,
                            lock.packages.get(&from),
                            &dep.name,
                            existing,
                        )
                    {
                        location = Some(candidate);
                    }
                    break;
                }
                if preserves_dependencies(&mut lock, &dependents, &candidate, &entry, &dep.name) {
                    location = Some(candidate);
                } else {
                    break;
                }
            }
            let Some(location) = location else {
                // A greedy placement cannot preserve previously resolved edges.
                // Let the joint resolver reconsider earlier placements.
                return peers::resolve(ctx, manifest, old, update).await;
            };
            if lock.packages.contains_key(&location) {
                continue;
            }
            if location.matches("node_modules/").count() > 128 {
                bail!("dependency nesting exceeds 128 levels");
            }
            let mut entry = entry;
            super::overrides::apply(manifest, lock.packages.get(&from), &dep.name, &mut entry)?;
            next.extend(super::bundles::insert(
                &mut lock, manifest, &location, &entry,
            )?);
            if dep.optional
                || lock
                    .packages
                    .get(&from)
                    .is_some_and(|v| flag(v, "optional"))
            {
                entry.set("optional", Value::Bool(true));
            }
            for dep in lockfile::dependencies(&entry, false)
                .into_iter()
                .filter(|d| !d.peer)
            {
                next.push((location.clone(), dep));
            }
            pipeline.prefetch(&entry, &dep.name);
            lock.packages.insert(location.clone(), entry);
            index_package(&lock, &mut dependents, &location);
            // `bundles::insert` adds packages directly; index the new subtree.
            let prefix = format!("{location}/");
            let bundled: Vec<String> = lock
                .packages
                .range(location.clone()..)
                .take_while(|(path, _)| {
                    path.as_str() == location.as_str() || path.starts_with(&prefix)
                })
                .map(|(path, _)| path.clone())
                .filter(|path| path != &location)
                .collect();
            for path in bundled {
                index_package(&lock, &mut dependents, &path);
            }
        }
        pending = next;
    }
    profile.mark("metadata_pipeline_ms");
    if !lock.finalize(manifest, ctx.peer_policy())? {
        ctx.progress
            .event("resolve", "checking peer dependency sets");
        // Keep the parallel fast path for peer-free trees. Peer sets need a joint
        // placement/version search rather than independent greedy hoisting.
        return peers::resolve(ctx, manifest, old, update).await;
    }
    profile.mark("peer_validation_ms");
    pipeline.finish().await;
    pipeline.report(&mut profile);
    profile.mark("prefetch_tail_ms");
    Ok(lock)
}
