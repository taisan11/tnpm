//! Successful package changes from before/after trees or lockfiles.
use crate::{
    cli::Options,
    json::Value,
    lockfile::{self, Lockfile},
    util::style::{self, Tone, progress},
};
use std::collections::BTreeSet;

pub(super) fn print(options: &Options, before: &Lockfile, after: &Lockfile) {
    progress::finish();
    let paths: BTreeSet<_> = before
        .packages
        .keys()
        .chain(after.packages.keys())
        .filter(|p| !p.is_empty())
        .collect();
    let mut changes = Vec::new();
    let (mut added, mut updated, mut removed) = (0, 0, 0);
    for path in paths {
        let old = before.packages.get(path);
        let new = after.packages.get(path);
        let from = old.and_then(|e| e.str("version"));
        let to = new.and_then(|e| e.str("version"));
        let names = old
            .and_then(|e| e.str("name"))
            .zip(new.and_then(|e| e.str("name")));
        let identity_changed = names.is_some_and(|(a, b)| a != b);
        if from == to && !identity_changed {
            continue;
        }
        if options.json {
            let mut change = Value::object();
            change.set("name", Value::from(lockfile::name_at(path)));
            change.set("from", from.map(Value::from).unwrap_or(Value::Null));
            change.set("to", to.map(Value::from).unwrap_or(Value::Null));
            change.set("location", Value::from(path.as_str()));
            changes.push(change);
        } else {
            let name = path.strip_prefix("node_modules/").unwrap_or(path);
            let line = match (from, to) {
                (None, Some(to)) => {
                    added += 1;
                    format!("+ {name}@{to}")
                }
                (Some(from), None) => {
                    removed += 1;
                    format!("- {name}@{from}")
                }
                (from, to) => {
                    updated += 1;
                    let (from, to) = (from.unwrap_or("?"), to.unwrap_or("?"));
                    if let Some((old, new)) = names.filter(|_| identity_changed) {
                        format!("~ {name} {old}@{from} -> {new}@{to}")
                    } else {
                        format!("~ {name} {from} -> {to}")
                    }
                }
            };
            println!("{}", progress::escape_controls(&line));
        }
    }
    if options.json {
        let mut result = Value::object();
        result.set("updated", Value::Number(changes.len().to_string()));
        result.set(
            "mode",
            Value::from(if options.lock_only && options.no_save {
                "planned"
            } else if options.lock_only {
                "lockfile"
            } else {
                "installed"
            }),
        );
        result.set("changes", Value::Array(changes));
        println!("{}", result.pretty());
    } else {
        let summary = if added + updated + removed == 0 {
            if options.command == "update" {
                "No packages were updated.".to_owned()
            } else {
                "No package changes.".to_owned()
            }
        } else {
            println!();
            let install = if options.lock_only {
                "added"
            } else {
                "installed"
            };
            let counts = [(install, added), ("updated", updated), ("removed", removed)]
                .into_iter()
                .filter(|&(_, count)| count != 0)
                .map(|(verb, count)| {
                    let unit = if count == 1 { "package" } else { "packages" };
                    format!("{verb} {count} {unit}")
                })
                .collect::<Vec<_>>()
                .join(", ");
            if options.lock_only && options.no_save {
                format!("Planned changes (not saved): {counts}")
            } else if options.lock_only {
                format!("Lockfile changes: {counts}")
            } else {
                counts
            }
        };
        println!("{}", style::stdout(&summary, Tone::Success));
    }
}
