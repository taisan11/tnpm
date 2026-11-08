//! Dependency lifecycle permissions. Only the project can extend the built-in list.
use crate::{cli::Options, json::Value, lockfile, scripts};
use anyhow::{Result, ensure};
use std::{collections::BTreeSet, path::Path};

const DEFAULTS: &str = include_str!("default-trusted-dependencies.txt");

pub(super) struct Trust {
    names: BTreeSet<String>,
}

impl Trust {
    pub(super) fn new(project: &Value, options: &Options) -> Result<Self> {
        let mut names: BTreeSet<_> = DEFAULTS.lines().map(str::to_owned).collect();
        if let Some(value) = project.get("trustedDependencies") {
            let Value::Array(values) = value else {
                anyhow::bail!("trustedDependencies must be an array of package names");
            };
            for value in values {
                let name = value.string().unwrap_or("");
                ensure!(
                    lockfile::valid_name(name),
                    "invalid trusted dependency: {}",
                    value.display()
                );
                names.insert(name.to_owned());
            }
        }
        names.extend(options.trust.iter().cloned());
        Ok(Self { names })
    }

    pub(super) fn allows(&self, entry: &Value, data: &Value) -> bool {
        // Project workspaces are first-party; never read permissions from dependencies.
        entry.get("link") == Some(&Value::Bool(true))
            || (entry.str("name") == data.str("name")
                && entry
                    .str("name")
                    .is_some_and(|name| self.names.contains(name)))
    }

    pub(super) fn has_scripts(directory: &Path, data: &Value, entry: &Value) -> bool {
        ["preinstall", "install", "postinstall"]
            .iter()
            .any(|event| data.get("scripts").and_then(|s| s.str(event)).is_some())
            || (entry.get("tnpmSpec").is_some()
                && data.get("scripts").and_then(|s| s.str("prepare")).is_some())
            || scripts::implicit_native(directory, data)
    }

    pub(super) fn warn(path: &str, data: &Value) {
        crate::util::style::progress::message(format_args!(
            "tnpm: blocked lifecycle scripts for {path}; review the package, then add {:?} to trustedDependencies or use rebuild --trust {:?}",
            data.str("name").unwrap_or(path),
            data.str("name").unwrap_or(path)
        ));
    }
}
