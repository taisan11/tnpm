//! Workspace discovery without following directory symlinks or entering node_modules.
use crate::{json::Value, lockfile, manifest, semver};
use anyhow::{Context, Result, bail};
use glob::{MatchOptions, Pattern};
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

struct WorkspacePattern {
    include: bool,
    matcher: Pattern,
    // Avoid walking unrelated subtrees for simple literal prefixes.
    literal_prefix: String,
}

fn matches(pattern: &WorkspacePattern, path: &str) -> bool {
    pattern.matcher.matches_path_with(
        Path::new(path),
        MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        },
    )
}

fn may_contain_match(pattern: &WorkspacePattern, dir: &str) -> bool {
    let prefix = &pattern.literal_prefix;
    prefix.is_empty()
        || dir == prefix
        || dir.starts_with(&format!("{prefix}/"))
        || prefix.starts_with(&format!("{dir}/"))
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
            let raw = p.string().context("workspace pattern must be a string")?;
            let (include, raw) = raw.strip_prefix('!').map_or((true, raw), |p| (false, p));
            let raw = raw.strip_prefix("./").unwrap_or(raw).trim_end_matches('/');
            if raw.is_empty()
                || raw.starts_with('/')
                || raw.contains(['\\', ':', '{', '}', '\0'])
                || raw.split('/').any(|s| s == ".." || s.is_empty())
            {
                bail!("unsafe or unsupported workspace pattern: {raw}");
            }
            let literal_prefix = raw
                .split('/')
                .take_while(|part| !part.contains(['*', '?', '[', ']']))
                .collect::<Vec<_>>()
                .join("/");
            Ok(WorkspacePattern {
                include,
                matcher: Pattern::new(raw)
                    .with_context(|| format!("invalid workspace glob: {raw}"))?,
                literal_prefix,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    fn walk(
        root: &Path,
        relative: &str,
        patterns: &[WorkspacePattern],
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
            if entry.path().join("package.json").is_file()
                && patterns.iter().any(|p| p.include && matches(p, &path))
                && !patterns.iter().any(|p| !p.include && matches(p, &path))
            {
                out.push(path.clone());
            }
            // Traverse using literal prefixes, but let glob handle actual matching.
            if patterns
                .iter()
                .any(|p| p.include && may_contain_match(p, &path))
            {
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
