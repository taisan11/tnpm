use crate::{cli::Options, json::Value, manifest::read as manifest, spec};
use anyhow::{Context, Result, bail};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
struct Package {
    name: String,
    version: String,
    path: PathBuf,
    manifest: Value,
    children: Vec<Package>,
    missing: bool,
    invalid: bool,
    extraneous: bool,
}

fn valid_name(name: &str) -> bool {
    let parts: Vec<_> = name.split('/').collect();
    let safe = |part: &str| {
        !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', '\0'])
    };
    match parts.as_slice() {
        [name] => !name.starts_with('@') && safe(name),
        [scope, name] => scope.starts_with('@') && scope.len() > 1 && safe(scope) && safe(name),
        _ => false,
    }
}

fn resolve(root: &Path, name: &str) -> Result<Option<PathBuf>> {
    if !valid_name(name) {
        bail!("invalid dependency name: {name}");
    }
    for ancestor in root.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|name| name == "node_modules")
        {
            continue;
        }
        let path = ancestor.join("node_modules").join(name);
        if path.join("package.json").is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn build(
    path: &Path,
    name: &str,
    data: Value,
    root: bool,
    stack: &mut HashSet<PathBuf>,
    used: &mut HashSet<PathBuf>,
) -> Result<Package> {
    let canonical = fs::canonicalize(path)?;
    used.insert(canonical.clone());
    let mut package = Package {
        name: data.str("name").unwrap_or(name).into(),
        version: data.str("version").unwrap_or("").into(),
        path: path.into(),
        manifest: data,
        children: Vec::new(),
        missing: false,
        invalid: false,
        extraneous: false,
    };
    if stack.contains(&canonical) {
        return Ok(package);
    }
    if stack.len() >= 128 {
        bail!("dependency nesting exceeds 128 levels");
    }
    stack.insert(canonical.clone());
    for dep in crate::lockfile::dependencies(&package.manifest, root) {
        let crate::lockfile::Dependency {
            name,
            spec,
            optional,
            peer,
            ..
        } = dep;
        let start = if peer && !root {
            let parent = path.parent().context("package has no parent")?;
            if parent
                .file_name()
                .is_some_and(|p| p.to_string_lossy().starts_with('@'))
            {
                parent
                    .parent()
                    .and_then(Path::parent)
                    .context("scoped package has no parent")?
            } else {
                parent.parent().context("package has no parent")?
            }
        } else {
            path
        };
        if let Some(resolved) = resolve(start, &name)? {
            let nested_peer = peer
                && !root
                && path
                    .join("node_modules")
                    .join(&name)
                    .join("package.json")
                    .is_file();
            let path = resolved;
            let data = manifest(&path)?.context("dependency disappeared while reading")?;
            let mut child = build(&path, &name, data, false, stack, used)?;
            child.invalid =
                nested_peer || spec::satisfies(&name, &spec, &child.manifest) == Some(false);
            child.name = name;
            package.children.push(child);
        } else if !optional {
            package.children.push(Package {
                name,
                version: spec,
                path: path.into(),
                manifest: Value::object(),
                children: Vec::new(),
                missing: true,
                invalid: false,
                extraneous: false,
            });
        }
    }
    stack.remove(&canonical);
    Ok(package)
}

fn installed(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut result = Vec::new();
    let entries = match fs::read_dir(root.join("node_modules")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') && entry.path().is_dir() {
            for child in fs::read_dir(entry.path())? {
                let child = child?;
                if child.path().join("package.json").is_file() {
                    result.push((
                        format!("{name}/{}", child.file_name().to_string_lossy()),
                        child.path(),
                    ));
                }
            }
        } else if entry.path().join("package.json").is_file() {
            result.push((name, entry.path()));
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn tree(root: &Path) -> Result<Package> {
    let data = manifest(root)?.unwrap_or_else(Value::object);
    let mut used = HashSet::new();
    let mut tree = build(
        root,
        root.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        data,
        true,
        &mut HashSet::new(),
        &mut used,
    )?;
    for (name, path) in installed(root)? {
        if !used.contains(&fs::canonicalize(&path)?) {
            let data = manifest(&path)?.context("dependency disappeared while reading")?;
            let mut child = build(&path, &name, data, false, &mut HashSet::new(), &mut used)?;
            child.name = name;
            child.extraneous = true;
            tree.children.push(child);
        }
    }
    tree.children.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tree)
}

fn matches(package: &Package, filters: &[String]) -> bool {
    filters.is_empty()
        || filters
            .iter()
            .any(|f| f == &package.name || f == &format!("{}@{}", package.name, package.version))
        || package.children.iter().any(|p| matches(p, filters))
}

fn tree_json(
    package: &Package,
    depth: usize,
    filters: &[String],
    problems: &mut Vec<Value>,
) -> Value {
    let mut value = Value::object();
    if !package.version.is_empty() {
        value.set("version", Value::from(package.version.as_str()));
    }
    for (key, flag) in [
        ("missing", package.missing),
        ("invalid", package.invalid),
        ("extraneous", package.extraneous),
    ] {
        if flag {
            value.set(key, Value::Bool(true));
            if key != "extraneous" {
                problems.push(Value::from(
                    format!("{key}: {}@{}", package.name, package.version).as_str(),
                ));
            }
        }
    }
    if depth > 0 {
        let mut deps = Value::object();
        for child in package.children.iter().filter(|p| matches(p, filters)) {
            deps.set(&child.name, tree_json(child, depth - 1, filters, problems));
        }
        if !deps.members().is_empty() {
            value.set("dependencies", deps);
        }
    }
    value
}

fn print_tree(package: &Package, prefix: &str, depth: usize, filters: &[String]) {
    if depth == 0 {
        return;
    }
    let children: Vec<_> = package
        .children
        .iter()
        .filter(|p| matches(p, filters))
        .collect();
    for (index, child) in children.iter().enumerate() {
        let last = index + 1 == children.len();
        let status = if child.missing {
            "UNMET DEPENDENCY "
        } else {
            ""
        };
        let suffix = if child.invalid {
            " invalid"
        } else if child.extraneous {
            " extraneous"
        } else {
            ""
        };
        println!(
            "{prefix}{}{status}{}@{}{suffix}",
            if last { "└── " } else { "├── " },
            child.name,
            child.version
        );
        print_tree(
            child,
            &format!("{prefix}{}", if last { "    " } else { "│   " }),
            depth - 1,
            filters,
        );
    }
}

pub fn list(root: &Path, options: &Options) -> Result<()> {
    let tree = tree(root)?;
    let depth = options
        .depth
        .unwrap_or(if options.all || !options.arguments.is_empty() {
            usize::MAX - 1
        } else {
            0
        })
        .saturating_add(1);
    let mut problems = Vec::new();
    let mut result = tree_json(&tree, depth, &options.arguments, &mut problems);
    result.set("name", Value::from(tree.name.as_str()));
    if !problems.is_empty() {
        result.set("problems", Value::Array(problems.clone()));
    }
    if options.json {
        println!("{}", result.pretty());
    } else {
        println!(
            "{}{} {}",
            tree.name,
            if tree.version.is_empty() {
                String::new()
            } else {
                format!("@{}", tree.version)
            },
            root.display()
        );
        if tree.children.iter().any(|p| matches(p, &options.arguments)) {
            print_tree(&tree, "", depth, &options.arguments);
        } else {
            println!("└── (empty)");
        }
    }
    if !problems.is_empty() {
        bail!("ELSPROBLEMS: missing or invalid dependencies");
    }
    if !options.arguments.is_empty()
        && !tree.children.iter().any(|p| matches(p, &options.arguments))
    {
        bail!("no matching installed dependencies");
    }
    Ok(())
}

fn funding_urls(value: &Value, urls: &mut Vec<String>) {
    match value {
        Value::String(url) => {
            if !urls.contains(url) {
                urls.push(url.clone());
            }
        }
        Value::Object(_) => {
            if let Some(url) = value.str("url") {
                funding_urls(&Value::from(url), urls);
            }
        }
        Value::Array(values) => {
            for value in values {
                funding_urls(value, urls);
            }
        }
        _ => {}
    }
}

fn collect_funding(package: &Package, seen: &mut HashSet<PathBuf>, entries: &mut Value) {
    if package.missing
        || !seen.insert(fs::canonicalize(&package.path).unwrap_or_else(|_| package.path.clone()))
    {
        return;
    }
    if let Some(funding) = package.manifest.get("funding") {
        let mut urls = Vec::new();
        funding_urls(funding, &mut urls);
        if !urls.is_empty() {
            let mut value = Value::object();
            value.set("version", Value::from(package.version.as_str()));
            value.set("funding", funding.clone());
            entries.set(&package.name, value);
        }
    }
    for child in &package.children {
        collect_funding(child, seen, entries);
    }
}

pub fn fund(root: &Path, options: &Options) -> Result<()> {
    let tree = tree(root)?;
    let mut entries = Value::object();
    let mut seen = HashSet::new();
    for child in &tree.children {
        collect_funding(child, &mut seen, &mut entries);
    }
    let mut result = Value::object();
    result.set("name", Value::from(tree.name.as_str()));
    result.set("version", Value::from(tree.version.as_str()));
    result.set("length", Value::Number(entries.members().len().to_string()));
    result.set("dependencies", entries.clone());
    if options.json {
        println!("{}", result.pretty());
    } else {
        println!("{}", tree.name);
        for (name, value) in entries.members() {
            let mut urls = Vec::new();
            funding_urls(value.get("funding").unwrap(), &mut urls);
            println!(
                "  {name}@{}\n    {}",
                value.str("version").unwrap_or(""),
                urls.join("\n    ")
            );
        }
        if entries.members().is_empty() {
            println!("  No dependencies are looking for funding");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn funding_accepts_all_npm_forms() {
        let value = Value::parse(
            r#"["https://a.test",{"type":"github","url":"https://b.test"},"https://a.test"]"#,
        )
        .unwrap();
        let mut urls = Vec::new();
        funding_urls(&value, &mut urls);
        assert_eq!(urls, ["https://a.test", "https://b.test"]);
    }

    #[test]
    fn dependency_names_cannot_escape_node_modules() {
        for name in ["../x", "@x/../../x", "/tmp/x", "foo\\bar", "@/x"] {
            assert!(!valid_name(name), "{name}");
        }
        assert!(valid_name("@scope/package"));
    }
}
