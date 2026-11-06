//! Physical dependency placement and the parallel peer-free resolution path.
use super::{CONCURRENCY, context::Context, flag};
use crate::{
    json::Value,
    lockfile::{self, Dependency, Lockfile},
    spec,
};
use anyhow::{Context as _, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{StreamExt, stream};
use rustc_hash::FxHashMap;
use std::collections::BTreeSet;

mod peers;

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

// A hoisted package must not shadow a previously satisfied dependency with an
// incompatible version. Only packages below the candidate's parent can see it.
fn preserves_dependencies(lock: &mut Lockfile, path: &str, entry: &Value, name: &str) -> bool {
    let scope = lockfile::parent(path);
    let affected: Vec<_> = lock
        .packages
        .iter()
        .filter(|(from, _)| {
            scope.is_empty()
                || *from == scope
                || from.starts_with(&format!("{scope}/node_modules/"))
        })
        .flat_map(|(from, data)| {
            lockfile::dependencies(data, from.is_empty())
                .into_iter()
                .filter(|dep| !dep.peer && dep.name == name)
                .filter(|dep| {
                    lock.resolve(from, name)
                        .is_some_and(|p| matches(&lock.packages[&p], name, &dep.spec))
                })
                .map(|dep| (from.clone(), dep))
        })
        .collect();
    lock.packages.insert(path.to_owned(), entry.clone());
    let valid = affected.iter().all(|(from, dep)| {
        lock.resolve(from, name)
            .is_some_and(|p| matches(&lock.packages[&p], name, &dep.spec))
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
    ctx.progress.event("resolve", "checking dependency graph");
    super::overrides::validate(manifest)?;
    let has_sources = manifest.get("workspaces").is_some()
        || old.is_some_and(|l| l.packages.values().any(|v| v.get("tnpmSpec").is_some()));
    if !update
        && !has_sources
        && let Some(old) = old
        && old.validate_with(manifest, ctx.peer_policy()).is_ok()
    {
        let mut lock = old.clone();
        let used = lock.reachable_with(manifest, true, ctx.peer_policy())?;
        lock.packages
            .retain(|p, _| p.is_empty() || used.contains(p));
        lock.packages
            .insert("".into(), lockfile::root_entry(manifest));
        lock.classify_with(manifest, ctx.peer_policy());
        ctx.progress.event(
            "resolve",
            &format!(
                "reusing {} locked packages",
                lock.packages.len().saturating_sub(1)
            ),
        );
        return Ok(lock);
    }
    let mut lock = Lockfile::default();
    lock.packages
        .insert("".into(), lockfile::root_entry(manifest));
    let mut pending: Vec<(String, Dependency)> = lockfile::dependencies(manifest, true)
        .into_iter()
        .filter(|d| !d.peer)
        .map(|d| (String::new(), d))
        .collect();
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
        let responses: Vec<_> = stream::iter(requests.into_iter().map(|(name, spec)| async move {
            let result = ctx
                .metadata(&name, &spec)
                .await
                .and_then(|v| descriptor(&v));
            ((name, spec), result)
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
        let mut errors = FxHashMap::default();
        for (key, result) in responses {
            match result {
                Ok(entry) => {
                    memo.insert(key, entry);
                }
                Err(error) => {
                    errors.insert(key, error);
                }
            }
        }
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
                        errors
                            .get(&key)
                            .map(|error| format!("{error:#}"))
                            .unwrap_or_else(|| "resolution failed".into())
                    ));
                    continue;
                }
                return Err(errors
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
                if preserves_dependencies(&mut lock, &candidate, &entry, &dep.name) {
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
            lock.packages.insert(location, entry);
        }
        pending = next;
    }
    let strict_policy = lockfile::PeerPolicy {
        strict: true,
        force: false,
        ..ctx.peer_policy()
    };
    if !lock.peer_conflicts(strict_policy).is_empty() {
        ctx.progress
            .event("resolve", "checking peer dependency sets");
        // Keep the parallel fast path for peer-free trees. Peer sets need a joint
        // placement/version search rather than independent greedy hoisting.
        return peers::resolve(ctx, manifest, old, update).await;
    }
    lock.classify_with(manifest, ctx.peer_policy());
    lock.reachable_with(manifest, true, ctx.peer_policy())?;
    Ok(lock)
}
