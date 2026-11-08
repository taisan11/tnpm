//! Registry package names and optional version/tag/range selectors.
use crate::{json::Value, semver};
use anyhow::{Result, bail};
use std::borrow::Cow;

pub fn safe_workspace_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "node_modules" && !p.starts_with('.'))
}

pub fn package_spec(spec: &str) -> Result<(&str, Option<&str>)> {
    let at = if let Some(unscoped) = spec.strip_prefix('@') {
        unscoped.find('@').map(|i| i + 1)
    } else {
        spec.find('@')
    };
    let (name, selector) = at
        .map(|i| (&spec[..i], Some(&spec[i + 1..])))
        .unwrap_or((spec, None));
    let parts: Vec<_> = name.split('/').collect();
    let valid = match parts.as_slice() {
        [name] => !name.starts_with('@') && !name.is_empty(),
        [scope, name] => scope.starts_with('@') && scope.len() > 1 && !name.is_empty(),
        _ => false,
    };
    if !valid
        || name.contains(['\\', '?', '#', '%', ':', '\0'])
        || name.chars().any(|c| c.is_whitespace() || c.is_control())
        || parts.iter().any(|p| *p == "." || *p == "..")
    {
        bail!("invalid registry package name: {name}");
    }
    Ok((name, selector.filter(|s| !s.is_empty())))
}

/// Keep the installation name separate from the package fetched from a registry.
pub fn dependency_target<'a>(name: &'a str, selector: &'a str) -> Result<(Cow<'a, str>, &'a str)> {
    if selector.starts_with("jsr:") {
        let jsr = crate::jsr::Specifier::parse(selector)?;
        if !jsr.subpath.is_empty() {
            bail!("JSR dependencies cannot select an export subpath");
        }
        return Ok((Cow::Owned(jsr.npm_name()), jsr.range));
    }
    if let Some(target) = selector.strip_prefix("npm:") {
        let (name, selector) = package_spec(target)?;
        let selector = selector.unwrap_or("latest");
        if selector.starts_with("npm:") {
            bail!("nested npm aliases are not supported");
        }
        Ok((Cow::Borrowed(name), selector))
    } else {
        Ok((Cow::Borrowed(name), selector))
    }
}

pub fn registry_dependency<'a>(
    name: &'a str,
    selector: &'a str,
) -> Result<(Cow<'a, str>, &'a str)> {
    let (target, range) = dependency_target(name, selector)?;
    if [
        "file:",
        "git",
        "http:",
        "https:",
        "npm:",
        "jsr:",
        "workspace:",
    ]
    .iter()
    .any(|prefix| range.starts_with(prefix))
        || range.contains('/')
    {
        bail!("unsupported dependency spec: {name}@{selector} (registry dependencies only)");
    }
    Ok((target, range))
}

pub fn matches_query(query: &str, location: &str, entry: &Value) -> Result<bool> {
    let path = query
        .strip_prefix("./")
        .unwrap_or(query)
        .trim_end_matches('/');
    if path.starts_with("node_modules/") {
        return Ok(path == location);
    }
    let (name, range) = package_spec(query)?;
    Ok(
        (crate::lockfile::name_at(location) == name || entry.str("name") == Some(name))
            && range.is_none_or(|r| {
                semver::satisfies(entry.str("version").unwrap_or(""), r) == Some(true)
            }),
    )
}

/// None denotes a tag/unsupported range; a different alias target never matches.
pub fn satisfies(name: &str, selector: &str, entry: &Value) -> Option<bool> {
    if let Some(range) = selector.strip_prefix("workspace:") {
        if entry.str("name") != Some(name) {
            return Some(false);
        }
        if matches!(range, "*" | "^" | "~") {
            return Some(true);
        }
        return semver::satisfies(entry.str("version").unwrap_or(""), range);
    }
    if let Some(source) = entry.str("tnpmSpec") {
        if selector == source {
            return Some(true);
        }
        if selector.starts_with("file:")
            || selector.starts_with("workspace:")
            || selector.starts_with("git")
            || selector.starts_with("github:")
            || selector.starts_with("https:")
            || selector.starts_with("http:")
            || selector.starts_with("ssh:")
        {
            return Some(false);
        }
    }
    let Ok((target, range)) = dependency_target(name, selector) else {
        return Some(false);
    };
    if (selector.starts_with("npm:") || selector.starts_with("jsr:"))
        && entry.str("name") != Some(target.as_ref())
    {
        return Some(false);
    }
    semver::satisfies(entry.str("version").unwrap_or(""), range)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_package_selectors() {
        assert_eq!(
            package_spec("@scope/pkg@next").unwrap(),
            ("@scope/pkg", Some("next"))
        );
        assert_eq!(package_spec("@scope/pkg").unwrap(), ("@scope/pkg", None));
        assert_eq!(package_spec("pkg@^1").unwrap(), ("pkg", Some("^1")));
        assert!(package_spec("../foo").is_err());
    }

    #[test]
    fn aliases_preserve_installation_names_and_validate_target_identity() {
        assert_eq!(
            package_spec("@local/pkg@npm:@jsr/owner__pkg@^1").unwrap(),
            ("@local/pkg", Some("npm:@jsr/owner__pkg@^1"))
        );
        assert_eq!(
            dependency_target("@local/pkg", "npm:@jsr/owner__pkg@^1").unwrap(),
            (Cow::Borrowed("@jsr/owner__pkg"), "^1")
        );
        assert_eq!(
            dependency_target("alias", "npm:real").unwrap(),
            (Cow::Borrowed("real"), "latest")
        );
        let entry = Value::parse(r#"{"name":"real","version":"1.2.0"}"#).unwrap();
        assert_eq!(satisfies("alias", "npm:real@^1", &entry), Some(true));
        assert_eq!(satisfies("alias", "npm:other@^1", &entry), Some(false));
        assert_eq!(satisfies("alias", "npm:real@^2", &entry), Some(false));
        assert!(dependency_target("alias", "npm:../bad").is_err());
        assert!(dependency_target("alias", "npm:real@npm:other").is_err());
    }

    #[test]
    fn jsr_selectors_preserve_registry_identity_and_validate_versions() {
        assert_eq!(
            dependency_target("alias", "jsr:@demo/math@^1").unwrap(),
            (Cow::Owned("@jsr/demo__math".into()), "^1")
        );
        let entry = Value::parse(r#"{"name":"@jsr/demo__math","version":"1.2.0"}"#).unwrap();
        assert_eq!(satisfies("alias", "jsr:@demo/math@^1", &entry), Some(true));
        assert_eq!(
            satisfies("alias", "jsr:@other/math@^1", &entry),
            Some(false)
        );
        assert_eq!(satisfies("alias", "jsr:@demo/math@^2", &entry), Some(false));
        assert!(dependency_target("alias", "jsr:@demo/math/extra").is_err());
    }
}
