//! Explain the locked physical graph without needing node_modules or network access.
use crate::{
    cli::Options,
    json::Value,
    lockfile::{self, Lockfile},
    spec,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::Path,
};

fn kind(dep: &lockfile::Dependency) -> &'static str {
    if dep.peer && dep.optional {
        "peerOptional"
    } else if dep.peer {
        "peer"
    } else if dep.optional {
        "optional"
    } else if dep.dev {
        "dev"
    } else {
        "prod"
    }
}

pub(crate) fn run(root: &Path, options: &Options) -> Result<()> {
    let lock =
        Lockfile::read(root)?.context("explain requires tnpm-lock.json; run install first")?;
    let mut incoming: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut predecessor = BTreeMap::new();
    let mut seen = BTreeSet::from([String::new()]);
    let mut queue = VecDeque::from([String::new()]);
    while let Some(from) = queue.pop_front() {
        let Some(data) = lock.packages.get(&from) else {
            continue;
        };
        for dep in lockfile::dependencies(data, from.is_empty()) {
            let Some(target) = lock.resolve_dependency(&from, &dep) else {
                continue;
            };
            let mut edge = Value::object();
            edge.set(
                "from",
                Value::from(if from.is_empty() { "root" } else { &from }),
            );
            edge.set("name", Value::from(dep.name.as_str()));
            edge.set("spec", Value::from(dep.spec.as_str()));
            edge.set("type", Value::from(kind(&dep)));
            incoming.entry(target.clone()).or_default().push(edge);
            if seen.insert(target.clone()) {
                predecessor.insert(target.clone(), from.clone());
                queue.push_back(target);
            }
        }
    }
    let mut selected = BTreeSet::new();
    for query in &options.arguments {
        let mut found = false;
        for (location, entry) in &lock.packages {
            if location.is_empty() {
                continue;
            }
            if spec::matches_query(query, location, entry)? {
                selected.insert(location.clone());
                found = true;
            }
        }
        if !found {
            bail!("no locked dependency matches: {query}");
        }
    }
    let mut output = Vec::new();
    for location in selected {
        let entry = &lock.packages[&location];
        let mut value =
            lockfile::copy_fields(entry, &["name", "version", "dev", "optional", "peer"]);
        value.set("location", Value::from(location.as_str()));
        let mut chain = vec![Value::from(location.as_str())];
        let mut at = &location;
        while let Some(parent) = predecessor.get(at) {
            chain.push(Value::from(if parent.is_empty() { "root" } else { parent }));
            at = parent;
        }
        chain.reverse();
        let dependents = incoming.remove(&location).unwrap_or_default();
        if !options.json {
            println!(
                "{}@{} ({location})",
                entry.str("name").unwrap_or(lockfile::name_at(&location)),
                entry.str("version").unwrap_or("?")
            );
            println!(
                "  {}",
                chain
                    .iter()
                    .map(Value::display)
                    .collect::<Vec<_>>()
                    .join(" -> ")
            );
            for edge in &dependents {
                println!(
                    "  {} requires {}@{} [{}]",
                    edge.str("from").unwrap(),
                    edge.str("name").unwrap(),
                    edge.str("spec").unwrap(),
                    edge.str("type").unwrap()
                );
            }
        }
        value.set("chain", Value::Array(chain));
        value.set("dependents", Value::Array(dependents));
        output.push(value);
    }
    if options.json {
        println!("{}", Value::Array(output).pretty());
    }
    Ok(())
}
