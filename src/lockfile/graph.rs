//! Borrowed, indexed snapshot for repeated dependency walks. JSON remains the
//! interchange format; traversal touches flat edge/offset/flag arrays instead
//! of cloning JSON trees and carrying owned physical paths through the queue.
use super::{Lockfile, PeerPolicy, parent, valid_name};
use crate::{json::Value, spec};
use anyhow::{Result, bail};
use rustc_hash::FxHashMap;
use std::collections::{BTreeSet, VecDeque};

pub(super) const DEV: u8 = 1;
pub(super) const OPTIONAL: u8 = 2;
pub(super) const PEER: u8 = 4;
pub(super) const DEV_OPTIONAL: u8 = 8;
const MISSING: usize = usize::MAX;

#[derive(Clone, Copy)]
struct Dependency<'a> {
    name: &'a str,
    spec: &'a str,
    flags: u8,
}

/// Flags include devOptional's propagation bit, not just serialized flags.
fn dependencies<'a>(data: &'a Value, root: bool, out: &mut Vec<Dependency<'a>>) {
    out.clear();
    for (field, flags) in [
        ("dependencies", 0),
        ("devDependencies", DEV | DEV_OPTIONAL),
        ("optionalDependencies", OPTIONAL | DEV_OPTIONAL),
    ] {
        if field == "devDependencies" && !root && data.get("tnpmWorkspace").is_none() {
            continue;
        }
        for (name, value) in data.get(field).map(Value::members).unwrap_or_default() {
            let Some(spec) = value.string() else {
                continue;
            };
            out.push(Dependency { name, spec, flags });
        }
    }
    // Stable sorting preserves map precedence and duplicate-key behavior.
    out.sort_by_key(|d| d.name);
    out.dedup_by(|later, earlier| {
        if later.name == earlier.name {
            *earlier = *later;
            true
        } else {
            false
        }
    });
    for (name, value) in data
        .get("peerDependencies")
        .map(Value::members)
        .unwrap_or_default()
    {
        if let Some(spec) = value.string() {
            let optional = data
                .get("peerDependenciesMeta")
                .and_then(|m| m.get(name))
                .and_then(|m| m.get("optional"))
                == Some(&Value::Bool(true));
            out.push(Dependency {
                name,
                spec,
                flags: PEER | if optional { OPTIONAL | DEV_OPTIONAL } else { 0 },
            });
        }
    }
}

pub(super) struct Graph<'a> {
    paths: Vec<&'a str>,
    data: Vec<&'a Value>,
    offsets: Vec<usize>,
    names: Vec<&'a str>,
    specs: Vec<&'a str>,
    edge_flags: Vec<u8>,
    targets: Vec<usize>,
}

impl<'a> Graph<'a> {
    pub(super) fn new(lock: &'a Lockfile, manifest: &'a Value) -> Self {
        Self::build(lock, manifest, None)
    }

    pub(super) fn selected(
        lock: &'a Lockfile,
        manifest: &'a Value,
        selected: &BTreeSet<String>,
    ) -> Self {
        Self::build(lock, manifest, Some(selected))
    }

    fn build(lock: &'a Lockfile, manifest: &'a Value, selected: Option<&BTreeSet<String>>) -> Self {
        let count = selected.map_or_else(
            || lock.packages.len() + usize::from(!lock.packages.contains_key("")),
            |s| s.len() + 1,
        );
        let mut paths = Vec::with_capacity(count);
        let mut data = Vec::with_capacity(count);
        paths.push("");
        data.push(manifest);
        for (path, entry) in &lock.packages {
            if !path.is_empty() && selected.is_none_or(|s| s.contains(path)) {
                paths.push(path.as_str());
                data.push(entry);
            }
        }
        let count = paths.len();
        let mut ids: FxHashMap<_, _> = paths.iter().enumerate().map(|(i, p)| (*p, i)).collect();
        // Preserve lookup scopes even if an external/filtered lockfile omits an
        // ancestor package. Virtual scopes are never dependency targets.
        for path in paths.iter().skip(1) {
            let mut at = parent(path);
            while !ids.contains_key(at) {
                ids.insert(at, ids.len());
                at = parent(at);
            }
        }
        let mut parents = vec![0; ids.len()];
        for (&path, &id) in &ids {
            if !path.is_empty() {
                parents[id] = ids[parent(path)];
            }
        }
        let mut children = FxHashMap::with_capacity_and_hasher(count, Default::default());
        for (id, path) in paths.iter().enumerate().skip(1) {
            children.insert((parents[id], super::name_at(path)), id);
        }
        let mut graph = Self {
            paths,
            data,
            offsets: Vec::with_capacity(count + 1),
            names: Vec::with_capacity(count.saturating_mul(4)),
            specs: Vec::with_capacity(count.saturating_mul(4)),
            edge_flags: Vec::with_capacity(count.saturating_mul(4)),
            targets: Vec::with_capacity(count.saturating_mul(4)),
        };
        let mut scratch = Vec::new();
        for (id, entry) in graph.data.iter().enumerate() {
            graph.offsets.push(graph.targets.len());
            dependencies(entry, id == 0, &mut scratch);
            for dep in &scratch {
                let mut at = if dep.flags & PEER != 0 && id != 0 {
                    parents[id]
                } else {
                    id
                };
                let target = if valid_name(dep.name) {
                    loop {
                        if let Some(&target) = children.get(&(at, dep.name)) {
                            break target;
                        }
                        if at == 0 {
                            break MISSING;
                        }
                        at = parents[at];
                    }
                } else {
                    MISSING
                };
                graph.names.push(dep.name);
                graph.specs.push(dep.spec);
                graph.edge_flags.push(dep.flags);
                graph.targets.push(target);
            }
        }
        graph.offsets.push(graph.targets.len());
        graph
    }

    pub(super) fn reachable(&self, strict: bool, policy: PeerPolicy) -> Result<BTreeSet<String>> {
        let mut seen = vec![false; self.paths.len()];
        let mut queue = VecDeque::from([0]);
        while let Some(from) = queue.pop_front() {
            for edge in self.offsets[from]..self.offsets[from + 1] {
                let flags = self.edge_flags[edge];
                if flags & PEER != 0 && policy.legacy {
                    continue;
                }
                let name = self.names[edge];
                if !valid_name(name) {
                    bail!("invalid dependency name: {name}");
                }
                let target = self.targets[edge];
                if target != MISSING {
                    if strict
                        && flags & PEER == 0
                        && spec::satisfies(name, self.specs[edge], self.data[target]) == Some(false)
                    {
                        bail!(
                            "lockfile version does not satisfy {}@{}",
                            name,
                            self.specs[edge]
                        );
                    }
                    if !seen[target] {
                        seen[target] = true;
                        queue.push_back(target);
                    }
                } else if strict
                    && flags & OPTIONAL == 0
                    && self.data[from].get("optional") != Some(&Value::Bool(true))
                {
                    bail!(
                        "lockfile is missing {} required by {}",
                        name,
                        self.paths[from]
                    );
                }
            }
        }
        Ok(self
            .paths
            .iter()
            .zip(seen)
            .filter(|(_, seen)| *seen)
            .map(|(path, _)| (*path).to_owned())
            .collect())
    }

    pub(super) fn classify(&self, policy: PeerPolicy) -> Vec<Option<u8>> {
        let mut flags = vec![None; self.paths.len()];
        let mut queue = VecDeque::from([(0, 0)]);
        while let Some((from, inherited)) = queue.pop_front() {
            for edge in self.offsets[from]..self.offsets[from + 1] {
                let edge_flags = self.edge_flags[edge];
                if edge_flags & PEER != 0 && policy.legacy {
                    continue;
                }
                let target = self.targets[edge];
                if target == MISSING {
                    continue;
                }
                let incoming = inherited | edge_flags;
                let next = flags[target].map_or(incoming, |old| old & incoming);
                if flags[target] != Some(next) {
                    flags[target] = Some(next);
                    queue.push_back((target, next));
                }
            }
        }
        flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::child;

    #[test]
    fn scoped_cycles_and_filtered_orphans_keep_their_physical_locations() {
        let manifest = Value::parse(
            r#"{"dependencies":{"@t/a":"1","shared":"1"},"devDependencies":{"tool":"1"}}"#,
        )
        .unwrap();
        let a = child("", "@t/a");
        let nested = child(&a, "@t/b");
        let shared = child("", "shared");
        let tool = child("", "tool");
        for stored_root in [false, true] {
            let mut lock = Lockfile::default();
            for (path, data) in [
                (
                    a.as_str(),
                    r#"{"version":"1.0.0","dependencies":{"@t/b":"1"}}"#,
                ),
                (
                    nested.as_str(),
                    r#"{"version":"1.0.0","dependencies":{"@t/a":"1"}}"#,
                ),
                ("node_modules/@t/b", r#"{"version":"2.0.0"}"#),
                (shared.as_str(), r#"{"version":"1.0.0"}"#),
                (
                    tool.as_str(),
                    r#"{"version":"1.0.0","dependencies":{"shared":"1"}}"#,
                ),
                (
                    "node_modules/absent/node_modules/shared",
                    r#"{"version":"9.0.0"}"#,
                ),
            ] {
                lock.packages
                    .insert(path.into(), Value::parse(data).unwrap());
            }
            if stored_root {
                lock.packages.insert("".into(), manifest.clone());
            }
            let expected =
                BTreeSet::from([a.clone(), nested.clone(), shared.clone(), tool.clone()]);
            assert_eq!(
                lock.reachable_with(&manifest, true, PeerPolicy::default())
                    .unwrap(),
                expected
            );
            lock.classify_with(&manifest, PeerPolicy::default());
            assert_eq!(lock.packages[&tool].get("dev"), Some(&Value::Bool(true)));
            assert!(lock.packages[&shared].get("dev").is_none());
            let mut selected = expected;
            selected.remove(&a);
            assert_eq!(
                lock.reachable_selected(&manifest, &selected).unwrap(),
                BTreeSet::from([shared.clone(), tool.clone()])
            );
            selected.remove(&shared);
            assert_eq!(
                lock.reachable_selected(&manifest, &selected).unwrap(),
                BTreeSet::from([tool.clone()])
            );
        }
    }

    #[test]
    fn borrowed_edges_preserve_overrides_and_validate_only_reachable_names() {
        let mut manifest = Value::parse(r#"{"dependencies":{"alias":"npm:real@^1","alias":"npm:real@~1.0","shared":"2"},"devDependencies":{"shared":"3"},"optionalDependencies":{"shared":"1"},"peerDependencies":{"shared":"1"}}"#).unwrap();
        let mut lock = Lockfile::default();
        lock.packages.insert(
            child("", "alias"),
            Value::parse(r#"{"name":"real","version":"1.0.0"}"#).unwrap(),
        );
        lock.packages.insert(
            child("", "shared"),
            Value::parse(r#"{"version":"1.0.0"}"#).unwrap(),
        );
        lock.packages.insert(
            child("", "unused"),
            Value::parse(r#"{"dependencies":{"../invalid":"1"}}"#).unwrap(),
        );
        let graph = Graph::new(&lock, &manifest);
        let end = graph.offsets[1];
        assert_eq!(&graph.names[..end], ["alias", "shared", "shared"]);
        assert_eq!(&graph.specs[..end], ["npm:real@~1.0", "1", "1"]);
        assert_eq!(&graph.edge_flags[..end], [0, OPTIONAL | DEV_OPTIONAL, PEER]);
        assert_eq!(
            graph.reachable(true, PeerPolicy::default()).unwrap(),
            BTreeSet::from([child("", "alias"), child("", "shared")])
        );
        manifest
            .get_mut("dependencies")
            .unwrap()
            .set("../invalid", Value::from("1"));
        assert!(
            Graph::new(&lock, &manifest)
                .reachable(true, PeerPolicy::default())
                .unwrap_err()
                .to_string()
                .contains("invalid dependency name")
        );
    }
}
