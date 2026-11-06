//! Workspace discovery without following directory symlinks or entering node_modules.
use crate::{json::Value, lockfile, manifest, semver};
use anyhow::{Context, Result, bail};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub struct Workspace {
    pub name: String,
    pub relative: String,
    pub manifest: Value,
}

fn pattern_matches(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.split_first(), path.split_first()) {
        (None, None) => true,
        (Some((&"**", rest)), _) => {
            pattern_matches(rest, path)
                || (!path.is_empty() && pattern_matches(pattern, &path[1..]))
        }
        (Some((part, rest)), Some((name, tail))) => {
            wildcard(part, name) && pattern_matches(rest, tail)
        }
        _ => false,
    }
}

fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < n.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == n[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

pub fn discover(root: &Path, data: &Value) -> Result<Vec<Workspace>> {
    let Some(patterns) = data.get("workspaces") else {
        return Ok(Vec::new());
    };
    let patterns = patterns.get("packages").unwrap_or(patterns);
    let Value::Array(patterns) = patterns else {
        bail!("workspaces must be an array of patterns");
    };
    let patterns = patterns
        .iter()
        .map(|p| {
            let p = p.string().context("workspace pattern must be a string")?;
            let p = p.strip_prefix("./").unwrap_or(p).trim_end_matches('/');
            if p.is_empty()
                || p.starts_with('/')
                || p.contains(['\\', ':', '{', '}', '[', ']'])
                || p.split('/').any(|s| s == ".." || s.is_empty())
            {
                bail!("unsafe or unsupported workspace pattern: {p}");
            }
            Ok(p)
        })
        .collect::<Result<Vec<_>>>()?;
    fn walk(
        root: &Path,
        relative: &str,
        patterns: &[&str],
        out: &mut Vec<String>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 {
            bail!("workspace nesting exceeds 64 levels");
        }
        let mut dirs = fs::read_dir(root.join(relative))?.collect::<std::io::Result<Vec<_>>>()?;
        dirs.sort_by_key(|e| e.file_name());
        for entry in dirs {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name == "node_modules" || !entry.file_type()?.is_dir() {
                continue;
            }
            let path = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let parts = path.split('/').collect::<Vec<_>>();
            if entry.path().join("package.json").is_file()
                && patterns.iter().any(|p| {
                    !p.starts_with('!')
                        && pattern_matches(&p.split('/').collect::<Vec<_>>(), &parts)
                })
                && !patterns
                    .iter()
                    .filter_map(|p| p.strip_prefix('!'))
                    .any(|p| pattern_matches(&p.split('/').collect::<Vec<_>>(), &parts))
            {
                out.push(path.clone());
            }
            // Only explore prefixes that can match a positive pattern.
            if patterns.iter().filter(|p| !p.starts_with('!')).any(|p| {
                let p = p.split('/').collect::<Vec<_>>();
                p.contains(&"**")
                    || (parts.len() < p.len() && parts.iter().zip(&p).all(|(n, p)| wildcard(p, n)))
            }) {
                walk(root, &path, patterns, out, depth + 1)?;
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    if !patterns.is_empty() {
        walk(root, "", &patterns, &mut paths, 0)?;
    }
    let mut names = BTreeSet::new();
    paths
        .into_iter()
        .map(|relative| {
            let manifest =
                manifest::read(&root.join(&relative))?.context("workspace package.json missing")?;
            let name = manifest
                .str("name")
                .context("workspace requires a name")?
                .to_owned();
            if !lockfile::valid_name(&name) || !names.insert(name.clone()) {
                bail!("invalid or duplicate workspace name: {name}");
            }
            let version = manifest.str("version").unwrap_or("0.0.0");
            if semver::Version::parse(version).is_none() {
                bail!("invalid workspace version: {name}@{version}");
            }
            Ok(Workspace {
                name,
                relative,
                manifest,
            })
        })
        .collect()
}

pub fn root(start: &Path) -> Result<Option<PathBuf>> {
    for candidate in start.ancestors() {
        if let Some(data) = manifest::read(candidate)?
            && data.get("workspaces").is_some()
            && discover(candidate, &data)?
                .iter()
                .any(|w| start.starts_with(candidate.join(&w.relative)))
        {
            return Ok(Some(candidate.to_owned()));
        }
    }
    Ok(None)
}

pub fn selected<'a>(items: &'a [Workspace], selectors: &[String]) -> Result<Vec<&'a Workspace>> {
    if selectors.is_empty() {
        return Ok(items.iter().collect());
    }
    let mut output = Vec::new();
    for selector in selectors {
        let selector = selector.trim_start_matches("./").trim_end_matches('/');
        let found = items
            .iter()
            .filter(|w| {
                w.name == selector
                    || w.relative == selector
                    || w.relative.starts_with(&format!("{selector}/"))
            })
            .collect::<Vec<_>>();
        if found.is_empty() {
            bail!("workspace not found: {selector}");
        }
        for item in found {
            if !output.iter().any(|w: &&Workspace| w.name == item.name) {
                output.push(item);
            }
        }
    }
    Ok(output)
}
