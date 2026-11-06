//! Root-only overrides. Active ancestor rules travel with descriptors, not hoisted paths.
use crate::{json::Value, lockfile, semver, spec};
use anyhow::{Context, Result, bail};

pub(super) fn validate(manifest: &Value) -> Result<()> {
    fn check(rules: &Value, root: &Value, direct: bool) -> Result<()> {
        if !matches!(rules, Value::Object(_)) {
            bail!("overrides must be an object");
        }
        for (key, rule) in rules.members() {
            if key == "." {
                if direct {
                    bail!("invalid root override '.'");
                }
                reference(
                    rule.string().context("override '.' must be a string")?,
                    root,
                )?;
                continue;
            }
            let (name, range) = spec::package_spec(key)?;
            let value = rule.string().or_else(|| rule.str("."));
            if let Some(value) = value {
                let value = reference(value, root)?;
                if direct
                    && let Some(dep) = lockfile::dependencies(root, true)
                        .iter()
                        .find(|d| !d.peer && d.name == name)
                    && range.is_none_or(|r| semver::intersects(r, &dep.spec) == Some(true))
                    && dep.spec != value
                {
                    bail!(
                        "EOVERRIDE: override for direct dependency {name} conflicts with {}",
                        dep.spec
                    );
                }
            }
            if matches!(rule, Value::Object(_)) {
                check(rule, root, false)?;
            } else if rule.string().is_none() {
                bail!("invalid override for {key}");
            }
        }
        Ok(())
    }
    if let Some(rules) = manifest.get("overrides") {
        check(rules, manifest, true)?;
    }
    Ok(())
}

fn reference<'a>(value: &'a str, root: &'a Value) -> Result<&'a str> {
    if let Some(name) = value.strip_prefix('$') {
        for field in ["dependencies", "devDependencies", "optionalDependencies"] {
            if let Some(value) = root.get(field).and_then(|d| d.str(name)) {
                return Ok(value);
            }
        }
        bail!("EOVERRIDE: unknown dependency reference {value}");
    }
    Ok(value)
}

fn matching<'a>(rules: &'a Value, name: &str, version: &str) -> Option<&'a Value> {
    rules.members().iter().find_map(|(key, rule)| {
        let (target, range) = spec::package_spec(key).ok()?;
        (target == name
            && range.is_none_or(|r| r == version || semver::intersects(version, r) == Some(true)))
        .then_some(rule)
    })
}

pub(super) fn scope(root: &Value, parent: Option<&Value>, name: &str, entry: &Value) -> Value {
    let mut scopes = match parent.and_then(|p| p.get("tnpmOverrideRules")) {
        Some(Value::Array(values)) => values.clone(),
        _ => root.get("overrides").cloned().into_iter().collect(),
    };
    if let Some(Value::Array(nodes)) = parent
        .and_then(|p| p.get("tnpmOverrideChildren"))
        .and_then(|p| p.get(name))
    {
        for node in nodes {
            let mut node = node.clone();
            node.remove(".");
            if !node.members().is_empty() && !scopes.contains(&node) {
                scopes.push(node);
            }
        }
        return Value::Array(scopes);
    }
    let inherited = scopes.clone();
    for rules in inherited {
        if let Some(rule) = matching(&rules, name, entry.str("version").unwrap_or(""))
            && matches!(rule, Value::Object(_))
        {
            let mut rule = rule.clone();
            rule.remove(".");
            if !rule.members().is_empty() && !scopes.contains(&rule) {
                scopes.push(rule);
            }
        }
    }
    Value::Array(scopes)
}

pub(super) fn apply(
    root: &Value,
    parent: Option<&Value>,
    name: &str,
    entry: &mut Value,
) -> Result<()> {
    let scopes = scope(root, parent, name, entry);
    apply_scoped(root, scopes, entry)
}

pub(super) fn apply_scoped(root: &Value, scopes: Value, entry: &mut Value) -> Result<()> {
    if let Value::Array(rules) = &scopes {
        let mut children = Value::object();
        for field in [
            "dependencies",
            "devDependencies",
            "optionalDependencies",
            "peerDependencies",
        ] {
            let Some(deps) = entry.get_mut(field) else {
                continue;
            };
            let values = deps.members().to_vec();
            for (name, original) in values {
                let version = original.string().unwrap_or("");
                let nodes = rules
                    .iter()
                    .filter_map(|r| matching(r, &name, version))
                    .collect::<Vec<_>>();
                let child_rules = nodes
                    .iter()
                    .filter(|r| matches!(r, Value::Object(_)))
                    .map(|r| (*r).clone())
                    .collect::<Vec<_>>();
                if !child_rules.is_empty() {
                    children.set(&name, Value::Array(child_rules));
                }
                let replacement = nodes
                    .iter()
                    .rev()
                    .find_map(|r| r.string().or_else(|| r.str(".")));
                if let Some(replacement) = replacement {
                    deps.set(&name, Value::from(reference(replacement, root)?));
                }
            }
        }
        if !rules.is_empty() {
            entry.set("tnpmOverrideRules", scopes);
        }
        if !children.members().is_empty() {
            entry.set("tnpmOverrideChildren", children);
        }
    }
    Ok(())
}

pub(super) fn same_scope(root: &Value, parent: Option<&Value>, name: &str, entry: &Value) -> bool {
    if root.get("overrides").is_none() {
        return true;
    }
    let expected = scope(root, parent, name, entry);
    entry
        .get("tnpmOverrideRules")
        .unwrap_or(&Value::Array(Vec::new()))
        == &expected
}
