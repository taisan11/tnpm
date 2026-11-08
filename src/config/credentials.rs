//! Minimal, atomic edits to user npmrc; never persist credentials to a project.
use super::Config;
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, io::Write, path::Path};

impl Config {
    pub fn credential_files(&self) -> impl Iterator<Item = &str> {
        self.values.iter().filter_map(|(key, value)| {
            matches!(
                key.rsplit(':').next().unwrap_or(key),
                "userconfig" | "globalconfig" | "keyfile" | "certfile"
            )
            .then_some(value.as_str())
        })
    }

    pub fn save_credentials(
        &self,
        scope: &str,
        token: Option<&str>,
        mapping: Option<(&str, &str)>,
    ) -> Result<()> {
        let path = Path::new(self.get("userconfig").context("missing userconfig")?);
        match fs::symlink_metadata(path) {
            Ok(meta) => ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "userconfig must be a regular file, not a symlink"
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("could not inspect userconfig"),
        }
        let old = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).context("could not read userconfig"),
        };
        let mut changes: BTreeMap<String, Option<String>> =
            ["_authToken", "_auth", "username", "_password", "email"]
                .into_iter()
                .map(|field| (format!("{scope}:{field}"), None))
                .collect();
        if let Some(token) = token {
            ensure!(
                !token.is_empty() && !token.chars().any(char::is_control) && !token.contains("${"),
                "invalid registry token"
            );
            changes.insert(
                format!("{scope}:_authToken"),
                Some(crate::json::Value::from(token).pretty()),
            );
        }
        if let Some((key, value)) = mapping {
            changes.insert(key.into(), Some(crate::json::Value::from(value).pretty()));
        }
        let mut text = String::new();
        for line in old.trim_start_matches('\u{feff}').lines() {
            let key = line
                .split_once('=')
                .map_or(line, |(key, _)| key)
                .trim()
                .trim_end_matches("[]");
            if !changes.contains_key(key) {
                text.push_str(line);
                text.push('\n');
            }
        }
        for (key, value) in changes {
            if let Some(value) = value {
                text.push_str(&format!("{key}={value}\n"));
            }
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        // NamedTempFile is created with mode 0600 on Unix, before any secret is written.
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|e| e.error)
            .context("could not save userconfig")?;
        Ok(())
    }
}
