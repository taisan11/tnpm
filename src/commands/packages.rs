use crate::{cli::Options, json::Value, manifest::read as manifest, spec};
use anyhow::{Context, Result, bail};
use rustc_hash::FxHashMap;
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    rc::Rc,
};

#[derive(Debug)]
struct Package {
    name: String,
    version: String,
    installed: Option<Rc<Installed>>,
    children: Vec<Package>,
    missing: bool,
    invalid: bool,
    extraneous: bool,
}

#[derive(Debug)]
struct Installed {
    canonical: Rc<Path>,
    data: Value,
}

struct Listing<'a> {
    root: &'a Path,
    manifest: &'a Value,
    lock: Option<&'a crate::lockfile::Lockfile>,
    canonical: FxHashMap<PathBuf, Rc<Path>>,
    packages: FxHashMap<PathBuf, Rc<Installed>>,
    module_dirs: FxHashMap<PathBuf, bool>,
}

impl Listing<'_> {
    fn canonical_path(&mut self, path: &Path) -> io::Result<Rc<Path>> {
        if let Some(canonical) = self.canonical.get(path) {
            return Ok(canonical.clone());
        }
        let canonical: Rc<Path> = self.resolve_canonical(path)?.into();
        self.canonical.insert(path.into(), canonical.clone());
        Ok(canonical)
    }

    fn resolve_canonical(&mut self, path: &Path) -> io::Result<PathBuf> {
        // On Linux, only symlinks need realpath once the parent is resolved.
        // Reuse the project and scope directories across sibling packages.
        #[cfg(target_os = "linux")]
        if path != self.root
            && path.starts_with(self.root)
            && let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        {
            let candidate = self.canonical_path(parent)?.join(name);
            if !fs::symlink_metadata(&candidate)?.file_type().is_symlink() {
                return Ok(candidate);
            }
            return fs::canonicalize(candidate);
        }
        fs::canonicalize(path)
    }

    fn package(&mut self, path: &Path) -> Result<Rc<Installed>> {
        if let Some(package) = self.packages.get(path) {
            return Ok(package.clone());
        }
        let mut data = manifest(path)?.context("dependency disappeared while reading")?;
        let entry = self.lock.and_then(|lock| {
            let location = path
                .strip_prefix(self.root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            lock.packages
                .get(&location)
                .filter(|entry| entry.str("version") == data.str("version"))
        });
        if let Some(entry) = entry {
            for field in ["tnpmWorkspace", "tnpmSpec"] {
                if let Some(value) = entry.get(field) {
                    data.set(field, value.clone());
                }
            }
        }
        crate::install::listing_overrides(self.manifest, entry, &mut data)?;
        let package = Rc::new(Installed {
            canonical: self.canonical_path(path)?,
            data,
        });
        self.packages.insert(path.into(), package.clone());
        Ok(package)
    }

    fn has_manifest(&self, path: &Path) -> bool {
        self.packages.contains_key(path) || path.join("package.json").is_file()
    }

    fn resolve(&mut self, start: &Path, name: &str) -> Result<Option<PathBuf>> {
        if !valid_name(name) {
            bail!("invalid dependency name: {name}");
        }
        for ancestor in start.ancestors() {
            if ancestor
                .file_name()
                .is_some_and(|name| name == "node_modules")
            {
                continue;
            }
            let dir = ancestor.join("node_modules");
            let exists = *self
                .module_dirs
                .entry(ancestor.into())
                .or_insert_with(|| dir.is_dir());
            if exists {
                let path = dir.join(name);
                if self.has_manifest(&path) {
                    return Ok(Some(path));
                }
            }
        }
        Ok(None)
    }
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

fn build(
    path: &Path,
    name: &str,
    installed: Rc<Installed>,
    root: bool,
    stack: &mut HashSet<Rc<Path>>,
    used: &mut HashSet<Rc<Path>>,
    listing: &mut Listing<'_>,
) -> Result<Package> {
    let Installed { canonical, data } = &*installed;
    used.insert(canonical.clone());
    let mut package = Package {
        name: data.str("name").unwrap_or(name).into(),
        version: data.str("version").unwrap_or("").into(),
        installed: Some(installed.clone()),
        children: Vec::new(),
        missing: false,
        invalid: false,
        extraneous: false,
    };
    if stack.contains(canonical) {
        return Ok(package);
    }
    if stack.len() >= 128 {
        bail!("dependency nesting exceeds 128 levels");
    }
    stack.insert(canonical.clone());
    for dep in crate::lockfile::dependencies(data, root) {
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
        if let Some(resolved) = listing.resolve(start, &name)? {
            let nested_peer = peer
                && !root
                && path
                    .join("node_modules")
                    .join(&name)
                    .join("package.json")
                    .is_file();
            let path = resolved;
            let installed = listing.package(&path)?;
            let mut child = build(&path, &name, installed.clone(), false, stack, used, listing)?;
            child.invalid =
                nested_peer || spec::satisfies(&name, &spec, &installed.data) == Some(false);
            child.name = name;
            package.children.push(child);
        } else if !optional {
            package.children.push(Package {
                name,
                version: spec,
                installed: None,
                children: Vec::new(),
                missing: true,
                invalid: false,
                extraneous: false,
            });
        }
    }
    stack.remove(canonical);
    Ok(package)
}

fn installed(root: &Path, listing: &Listing<'_>) -> Result<Vec<(String, PathBuf)>> {
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
                if listing.has_manifest(&child.path()) {
                    result.push((
                        format!("{name}/{}", child.file_name().to_string_lossy()),
                        child.path(),
                    ));
                }
            }
        } else if listing.has_manifest(&entry.path()) {
            result.push((name, entry.path()));
        }
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

fn tree(root: &Path) -> Result<Package> {
    let data = manifest(root)?.unwrap_or_else(Value::object);
    let data = crate::install::workspace_manifest(root, &data)?;
    let lock = crate::lockfile::Lockfile::read(root)?;
    let mut listing = Listing {
        root,
        manifest: &data,
        lock: lock.as_ref(),
        canonical: FxHashMap::default(),
        packages: FxHashMap::default(),
        module_dirs: FxHashMap::default(),
    };
    let project = Rc::new(Installed {
        canonical: listing.canonical_path(root)?,
        data: data.clone(),
    });
    let mut used = HashSet::new();
    let mut tree = build(
        root,
        root.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        project,
        true,
        &mut HashSet::new(),
        &mut used,
        &mut listing,
    )?;
    for (name, path) in installed(root, &listing)? {
        if !used.contains(&listing.canonical_path(&path)?) {
            let installed = listing.package(&path)?;
            let mut child = build(
                &path,
                &name,
                installed,
                false,
                &mut HashSet::new(),
                &mut used,
                &mut listing,
            )?;
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

fn print_tree(
    output: &mut impl Write,
    package: &Package,
    prefix: &str,
    depth: usize,
    filters: &[String],
) -> io::Result<bool> {
    let mut problems = package.missing || package.invalid;
    if depth == 0 {
        return Ok(problems);
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
        writeln!(
            output,
            "{prefix}{}{status}{}@{}{suffix}",
            if last { "└── " } else { "├── " },
            child.name,
            child.version
        )?;
        problems |= print_tree(
            output,
            child,
            &format!("{prefix}{}", if last { "    " } else { "│   " }),
            depth - 1,
            filters,
        )?;
    }
    Ok(problems)
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
    let matched = tree.children.iter().any(|p| matches(p, &options.arguments));
    let has_problems = if options.json {
        let mut problems = Vec::new();
        let mut result = tree_json(&tree, depth, &options.arguments, &mut problems);
        result.set("name", Value::from(tree.name.as_str()));
        let has_problems = !problems.is_empty();
        if has_problems {
            result.set("problems", Value::Array(problems));
        }
        println!("{}", result.pretty());
        has_problems
    } else {
        let mut output = io::BufWriter::new(io::stdout().lock());
        writeln!(
            output,
            "{}{} {}",
            tree.name,
            if tree.version.is_empty() {
                String::new()
            } else {
                format!("@{}", tree.version)
            },
            root.display()
        )?;
        let has_problems = if matched {
            print_tree(&mut output, &tree, "", depth, &options.arguments)?
        } else {
            writeln!(output, "└── (empty)")?;
            false
        };
        output.flush()?;
        has_problems
    };
    if has_problems {
        bail!("ELSPROBLEMS: missing or invalid dependencies");
    }
    if !options.arguments.is_empty() && !matched {
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

fn collect_funding(package: &Package, seen: &mut HashSet<Rc<Path>>, entries: &mut Value) {
    let Some(installed) = &package.installed else {
        return;
    };
    if !seen.insert(installed.canonical.clone()) {
        return;
    }
    if let Some(funding) = installed.data.get("funding") {
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
