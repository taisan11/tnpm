//! Lockfiles describe physical package locations, not a nested logical tree.
use crate::{fs::write_text, json::Value, spec};
use anyhow::{Context, Result, bail};
use nojson::{DisplayJson, JsonFormatter};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

mod graph;

pub const ROOT_FIELDS: &[&str] = &[
    "name",
    "version",
    "license",
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
    "peerDependenciesMeta",
    "engines",
    "workspaces",
    "overrides",
];
pub const DEP_FIELDS: &[&str] = &[
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub spec: String,
    pub dev: bool,
    pub optional: bool,
    pub peer: bool,
}

pub fn dependencies(manifest: &Value, root: bool) -> Vec<Dependency> {
    let mut deps = BTreeMap::new();
    for field in ["dependencies", "devDependencies", "optionalDependencies"] {
        if field == "devDependencies" && !root && manifest.get("tnpmWorkspace").is_none() {
            continue;
        }
        for (name, value) in manifest.get(field).map(Value::members).unwrap_or_default() {
            if let Some(spec) = value.string() {
                deps.insert(
                    name.clone(),
                    Dependency {
                        name: name.clone(),
                        spec: spec.into(),
                        dev: field == "devDependencies",
                        optional: field == "optionalDependencies",
                        peer: false,
                    },
                );
            }
        }
    }
    let mut deps: Vec<_> = deps.into_values().collect();
    for (name, value) in manifest
        .get("peerDependencies")
        .map(Value::members)
        .unwrap_or_default()
    {
        if let Some(spec) = value.string() {
            deps.push(Dependency {
                name: name.clone(),
                spec: spec.into(),
                dev: false,
                peer: true,
                optional: manifest
                    .get("peerDependenciesMeta")
                    .and_then(|m| m.get(name))
                    .and_then(|m| m.get("optional"))
                    == Some(&Value::Bool(true)),
            });
        }
    }
    deps
}

/// Peer version mismatches warn by default; presence and placement remain required.
#[derive(Clone, Copy, Debug, Default)]
pub struct PeerPolicy {
    pub legacy: bool,
    pub strict: bool,
    pub force: bool,
}

impl PeerPolicy {
    pub fn allows_mismatch(self) -> bool {
        self.force || !self.strict
    }
}

pub fn peer_error(from: &str, dep: &Dependency, found: Option<&Value>) -> String {
    format!(
        "ERESOLVE: {} requires {}peer {}@{}, found {}",
        if from.is_empty() { "root" } else { from },
        if dep.optional { "optional " } else { "" },
        dep.name,
        dep.spec,
        found
            .and_then(|v| v.str("version"))
            .unwrap_or("no valid peer at or above its parent")
    )
}

pub fn root_entry(manifest: &Value) -> Value {
    copy_fields(manifest, ROOT_FIELDS)
}
pub fn copy_fields(value: &Value, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|key| value.get(key).map(|v| ((*key).into(), v.clone())))
            .collect(),
    )
}

pub fn valid_name(name: &str) -> bool {
    spec::package_spec(name).is_ok_and(|(n, selector)| n == name && selector.is_none())
        && !name.chars().any(|c| c.is_control())
}

pub fn valid_location(path: &str) -> bool {
    path.strip_prefix("node_modules/")
        .is_some_and(|p| p.split("/node_modules/").all(valid_name))
}

pub fn name_at(path: &str) -> &str {
    path.rsplit_once("node_modules/")
        .map(|(_, n)| n)
        .unwrap_or(path)
}
pub fn parent(path: &str) -> &str {
    path.rsplit_once("/node_modules/")
        .map(|(p, _)| p)
        .unwrap_or("")
}
pub fn child(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        format!("node_modules/{name}")
    } else {
        format!("{parent}/node_modules/{name}")
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lockfile {
    pub packages: BTreeMap<String, Value>,
}

struct CompactEntry<'a>(&'a Value);
struct ExtraFields<'a>(&'a Value);

impl DisplayJson for ExtraFields<'_> {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            for (key, value) in self.0.members() {
                if ["version", "resolved", "integrity"].contains(&key.as_str())
                    && matches!(value, Value::String(_))
                {
                    continue;
                }
                f.member(key, value)?;
            }
            Ok(())
        })
    }
}

impl DisplayJson for CompactEntry<'_> {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        let indent = f.get_indent_size();
        let spacing = f.get_spacing();
        f.set_indent_size(0);
        f.set_spacing(false);
        let result = f.array(|f| {
            for key in ["version", "resolved", "integrity"] {
                f.element(
                    self.0
                        .get(key)
                        .filter(|v| matches!(v, Value::String(_)))
                        .unwrap_or(&Value::Null),
                )?;
            }
            f.element(ExtraFields(self.0))
        });
        f.set_indent_size(indent);
        f.set_spacing(spacing);
        result
    }
}

struct PackageEntries<'a>(&'a BTreeMap<String, Value>);
impl DisplayJson for PackageEntries<'_> {
    fn fmt(&self, f: &mut JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            for (path, entry) in self.0 {
                if path.is_empty() {
                    f.member(path, entry)?;
                } else {
                    f.member(path, CompactEntry(entry))?;
                }
            }
            Ok(())
        })
    }
}

fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("could not inspect {}", path.display())),
    }
}

impl Lockfile {
    pub fn read(root: &Path) -> Result<Option<Self>> {
        let path = root.join("tnpm-lock.json");
        if !exists(&path)? {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let value =
            Value::parse(&text).with_context(|| format!("could not parse {}", path.display()))?;
        if !matches!(value.get("tnpmLockfileVersion"), Some(Value::Number(v)) if v == "1") {
            bail!("only tnpm-lock.json tnpmLockfileVersion 1 is supported");
        }
        let Value::Object(fields) = value else {
            bail!("lockfile must be an object")
        };
        let Some(Value::Object(entries)) = fields
            .into_iter()
            .find_map(|(k, v)| (k == "packages").then_some(v))
        else {
            bail!("lockfile has no packages object")
        };
        let mut packages = BTreeMap::new();
        for (path, mut entry) in entries {
            if !path.is_empty() {
                let Value::Array(parts) = entry else {
                    bail!("invalid compact package: {path}")
                };
                let [version, resolved, integrity, extra]: [Value; 4] = parts
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid compact package: {path}"))?;
                let Value::Object(mut fields) = extra else {
                    bail!("invalid compact package: {path}")
                };
                for (key, value) in [
                    ("version", version),
                    ("resolved", resolved),
                    ("integrity", integrity),
                ] {
                    if value != Value::Null {
                        if !matches!(value, Value::String(_))
                            || fields.iter().any(|(k, _)| k == key)
                        {
                            bail!("invalid compact field {key}: {path}");
                        }
                        fields.push((key.into(), value));
                    }
                }
                entry = Value::Object(fields);
            }
            if (!path.is_empty() && !valid_location(&path)) || !matches!(entry, Value::Object(_)) {
                bail!("invalid lockfile package location: {path}");
            }
            if entry.get("link") == Some(&Value::Bool(true))
                && !entry
                    .str("tnpmWorkspace")
                    .is_some_and(spec::safe_workspace_path)
            {
                bail!("invalid workspace link: {path}");
            }
            if packages.insert(path.clone(), entry).is_some() {
                bail!("duplicate lockfile package location: {path}");
            }
        }
        Ok(Some(Self { packages }))
    }

    pub fn value(&self, manifest: &Value, hidden: bool) -> Value {
        let mut value = copy_fields(manifest, &["name", "version"]);
        value.set("tnpmLockfileVersion", Value::Number("1".into()));
        value.set("requires", Value::Bool(true));
        value.set(
            "packages",
            Value::Object(
                self.packages
                    .iter()
                    .filter(|(p, _)| !hidden || !p.is_empty())
                    .map(|(p, v)| (p.clone(), v.clone()))
                    .collect(),
            ),
        );
        value
    }

    pub fn write(&self, root: &Path, manifest: &Value) -> Result<()> {
        let text = nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.object(|f| {
                for key in ["name", "version"] {
                    if let Some(value) = manifest.get(key) {
                        f.member(key, value)?;
                    }
                }
                f.member("tnpmLockfileVersion", 1)?;
                f.member("packages", PackageEntries(&self.packages))
            })
        })
        .to_string();
        write_text(&root.join("tnpm-lock.json"), &(text + "\n"))
    }

    pub fn resolve(&self, from: &str, name: &str) -> Option<String> {
        if !valid_name(name) {
            return None;
        }
        let mut at = from;
        loop {
            let path = child(at, name);
            if self.packages.contains_key(&path) {
                return Some(path);
            }
            if at.is_empty() {
                return None;
            }
            at = parent(at);
        }
    }

    pub fn resolve_dependency(&self, from: &str, dep: &Dependency) -> Option<String> {
        if dep.peer && !from.is_empty() {
            self.resolve(parent(from), &dep.name)
        } else {
            self.resolve(from, &dep.name)
        }
    }

    pub fn peer_conflicts(&self, policy: PeerPolicy) -> Vec<String> {
        if policy.legacy {
            return Vec::new();
        }
        let mut errors = Vec::new();
        for (from, data) in &self.packages {
            for dep in dependencies(data, from.is_empty())
                .into_iter()
                .filter(|d| d.peer)
            {
                let path = self.resolve_dependency(from, &dep);
                let found = path.as_ref().map(|p| &self.packages[p]);
                let nested =
                    !from.is_empty() && self.packages.contains_key(&child(from, &dep.name));
                let invalid = nested
                    || found
                        .is_some_and(|v| spec::satisfies(&dep.name, &dep.spec, v) == Some(false));
                if (invalid || (found.is_none() && !dep.optional))
                    && (!policy.allows_mismatch() || found.is_none() || nested)
                {
                    errors.push(peer_error(from, &dep, found));
                }
            }
        }
        errors
    }

    pub fn peer_warnings(&self, policy: PeerPolicy) -> Vec<String> {
        if policy.legacy || !policy.allows_mismatch() {
            return Vec::new();
        }
        self.packages
            .iter()
            .flat_map(|(from, data)| {
                dependencies(data, from.is_empty())
                    .into_iter()
                    .filter_map(move |dep| {
                        if !dep.peer {
                            return None;
                        }
                        let path = self.resolve_dependency(from, &dep)?;
                        let entry = &self.packages[&path];
                        (spec::satisfies(&dep.name, &dep.spec, entry) == Some(false))
                            .then(|| peer_error(from, &dep, Some(entry)))
                    })
            })
            .collect()
    }

    pub fn validate_with(&self, manifest: &Value, policy: PeerPolicy) -> Result<()> {
        let locked = self
            .packages
            .get("")
            .context("lockfile has no root package")?;
        for field in DEP_FIELDS.iter().chain([&"peerDependenciesMeta"]) {
            // Object order is irrelevant for dependency maps.
            let map = |v: &Value| -> BTreeMap<String, Value> {
                v.get(field)
                    .map(Value::members)
                    .unwrap_or_default()
                    .iter()
                    .cloned()
                    .collect()
            };
            if map(locked) != map(manifest) {
                bail!("package.json and lockfile disagree on {field}; run install first");
            }
        }
        for field in ["overrides", "workspaces"] {
            if locked.get(field) != manifest.get(field) {
                bail!("package.json and lockfile disagree on {field}; run install first");
            }
        }
        self.reachable_with(manifest, true, policy)?;
        Ok(())
    }

    /// Walk only selected physical locations without copying/filtering JSON.
    pub fn reachable_selected(
        &self,
        manifest: &Value,
        selected: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>> {
        graph::Graph::selected(self, manifest, selected).reachable(false, PeerPolicy::default())
    }

    pub fn reachable_with(
        &self,
        manifest: &Value,
        strict: bool,
        policy: PeerPolicy,
    ) -> Result<BTreeSet<String>> {
        if strict && let Some(error) = self.peer_conflicts(policy).first() {
            bail!("{error}");
        }
        graph::Graph::new(self, manifest).reachable(strict, policy)
    }

    pub fn classify_with(&mut self, manifest: &Value, policy: PeerPolicy) {
        // A shared production dependency is not dev-only. Graph snapshots borrow
        // JSON only until traversal finishes; mutation cannot invalidate edges.
        let flags = graph::Graph::new(self, manifest).classify(policy);
        let mut flags = flags.into_iter();
        if !self.packages.contains_key("") {
            flags.next(); // The indexed graph always has a synthetic root.
        }
        for data in self.packages.values_mut() {
            for flag in ["dev", "optional", "devOptional", "peer"] {
                data.remove(flag);
            }
            if let Some(bits) = flags.next().flatten() {
                if bits & graph::DEV != 0 {
                    data.set("dev", Value::Bool(true));
                }
                if bits & graph::OPTIONAL != 0 {
                    data.set("optional", Value::Bool(true));
                }
                if bits & graph::PEER != 0 {
                    data.set("peer", Value::Bool(true));
                }
                if bits & graph::DEV_OPTIONAL != 0 && bits & (graph::DEV | graph::OPTIONAL) == 0 {
                    data.set("devOptional", Value::Bool(true));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_are_confined_to_node_modules() {
        for p in [
            "node_modules/a",
            "node_modules/@s/a",
            "node_modules/a/node_modules/@s/b",
        ] {
            assert!(valid_location(p), "{p}");
        }
        for p in [
            "/node_modules/a",
            "node_modules/../x",
            "node_modules/a/../../x",
            "node_modules/a/node_modules/../../x",
            "node_modules/a\\b",
        ] {
            assert!(!valid_location(p), "{p}");
        }
    }
    #[test]
    fn nearest_ancestor_wins() {
        let mut lock = Lockfile::default();
        for p in [
            "node_modules/a",
            "node_modules/b",
            "node_modules/a/node_modules/b",
        ] {
            lock.packages.insert(p.into(), Value::object());
        }
        assert_eq!(
            lock.resolve("node_modules/a", "b").as_deref(),
            Some("node_modules/a/node_modules/b")
        );
        assert_eq!(
            lock.resolve("node_modules/a/node_modules/b", "a")
                .as_deref(),
            Some("node_modules/a")
        );
    }

    #[test]
    fn mixed_dev_optional_and_peer_paths_propagate_to_children() {
        let manifest =
            Value::parse(r#"{"devDependencies":{"dev":"1"},"optionalDependencies":{"opt":"1"}}"#)
                .unwrap();
        let mut lock = Lockfile::default();
        lock.packages.insert("".into(), root_entry(&manifest));
        for (name, data) in [
            (
                "dev",
                r#"{"version":"1.0.0","peerDependencies":{"shared":"1"}}"#,
            ),
            (
                "opt",
                r#"{"version":"1.0.0","peerDependencies":{"shared":"1"}}"#,
            ),
            (
                "shared",
                r#"{"version":"1.0.0","dependencies":{"child":"1"}}"#,
            ),
            ("child", r#"{"version":"1.0.0"}"#),
        ] {
            lock.packages
                .insert(child("", name), Value::parse(data).unwrap());
        }
        lock.classify_with(&manifest, PeerPolicy::default());
        for name in ["shared", "child"] {
            let entry = &lock.packages[&child("", name)];
            assert_eq!(entry.get("devOptional"), Some(&Value::Bool(true)));
            assert_eq!(entry.get("peer"), Some(&Value::Bool(true)));
            assert!(entry.get("dev").is_none());
            assert!(entry.get("optional").is_none());
        }
    }
}
