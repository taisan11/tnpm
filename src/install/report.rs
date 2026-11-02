//! Successful update results; installed updates use actual before/after trees.
use crate::{
    cli::Options,
    json::Value,
    lockfile::{self, Lockfile},
    util::style::table,
};
use std::collections::BTreeSet;

pub(super) fn print(options: &Options, before: &Lockfile, after: &Lockfile) {
    crate::util::style::progress::finish();
    let paths: BTreeSet<_> = before
        .packages
        .keys()
        .chain(after.packages.keys())
        .filter(|p| !p.is_empty())
        .collect();
    let mut changes = Vec::new();
    let mut rows = Vec::new();
    for path in paths {
        let old = before.packages.get(path);
        let new = after.packages.get(path);
        let from = old.and_then(|e| e.str("version"));
        let to = new.and_then(|e| e.str("version"));
        let identity_changed = old
            .and_then(|e| e.str("name"))
            .zip(new.and_then(|e| e.str("name")))
            .is_some_and(|(a, b)| a != b);
        if from == to && !identity_changed {
            continue;
        }
        let mut change = Value::object();
        change.set("name", Value::from(lockfile::name_at(path)));
        change.set("from", from.map(Value::from).unwrap_or(Value::Null));
        change.set("to", to.map(Value::from).unwrap_or(Value::Null));
        change.set("location", Value::from(path.as_str()));
        changes.push(change);
        rows.push([
            path.strip_prefix("node_modules/")
                .unwrap_or(path)
                .to_owned(),
            from.unwrap_or("MISSING").to_owned(),
            to.unwrap_or("REMOVED").to_owned(),
        ]);
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
    } else if rows.is_empty() {
        println!(
            "{}",
            crate::util::style::stdout(
                "No packages were updated.",
                crate::util::style::Tone::Success
            )
        );
    } else {
        print!("{}", table::format(["Package", "Before", "After"], &rows));
        if options.lock_only && options.no_save {
            println!("Planned {} package updates (not saved).", rows.len());
        } else {
            println!(
                "Updated {} package{}{}.",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" },
                if options.lock_only {
                    " in lockfile"
                } else {
                    ""
                }
            );
        }
    }
}
