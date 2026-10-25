//! Per-command configuration, HTTP clients, and metadata memoization.
use super::flag;
use crate::{
    cli::Options, config::Config, json::Value, lockfile, progress::Progress, registry, scripts,
    spec,
};
use anyhow::{Context as _, Result, bail};
use rustc_hash::FxHashMap;
use std::{cell::RefCell, rc::Rc};

type PackageVersions = FxHashMap<(String, String, bool), Rc<Vec<Value>>>;

pub(super) struct Context<'a> {
    pub(super) options: &'a Options,
    pub(super) config: &'a Config,
    pub(super) progress: Progress,
    clients: RefCell<FxHashMap<(String, bool), reqwest::Client>>,
    versions: RefCell<PackageVersions>,
}

impl<'a> Context<'a> {
    pub(super) fn new(options: &'a Options, config: &'a Config) -> Self {
        Self {
            options,
            config,
            progress: Progress::new(matches!(
                options.command.as_str(),
                "install" | "update" | "ci" | "uninstall" | "exec" | "create"
            )),
            clients: RefCell::new(FxHashMap::default()),
            versions: RefCell::new(FxHashMap::default()),
        }
    }

    pub(super) fn offline(&self) -> bool {
        self.options.offline || self.config.enabled("offline")
    }
    pub(super) fn peer_policy(&self) -> lockfile::PeerPolicy {
        lockfile::PeerPolicy {
            legacy: self.options.legacy_peer_deps || self.config.enabled("legacy-peer-deps"),
            strict: self.options.strict_peer_deps || self.config.enabled("strict-peer-deps"),
            force: self.options.force || self.config.enabled("force"),
        }
    }
    pub(super) fn warn_peers(&self, lock: &lockfile::Lockfile) {
        for warning in lock.peer_warnings(self.peer_policy()) {
            crate::progress::message(format_args!(
                "tnpm: warning: overriding peer dependency: {}",
                warning.strip_prefix("ERESOLVE: ").unwrap_or(&warning)
            ));
        }
    }
    pub(super) fn lifecycle(
        &self,
        root: &std::path::Path,
        manifest: &Value,
        event: &str,
    ) -> Result<()> {
        if manifest.get("scripts").and_then(|s| s.str(event)).is_none() {
            return Ok(());
        }
        let label = format!("{} ({event})", root.display());
        self.progress.start("scripts", &label);
        let result = scripts::lifecycle_output(root, manifest, event, self.options.json);
        self.progress.finish(
            "scripts",
            &label,
            if result.is_ok() {
                "completed"
            } else {
                "failed"
            },
        );
        result
    }
    pub(super) fn client(&self, url: &reqwest::Url, tarball: bool) -> Result<reqwest::Client> {
        let key = (self.config.http_client_key(url), tarball);
        if let Some(client) = self.clients.borrow().get(&key) {
            return Ok(client.clone());
        }
        let client = self.config.http_client(url, tarball)?;
        self.clients.borrow_mut().insert(key, client.clone());
        Ok(client)
    }

    pub(super) async fn metadata(&self, name: &str, spec: &str) -> Result<Value> {
        self.metadata_selected(name, spec, true)
            .await?
            .last()
            .cloned()
            .with_context(|| format!("no matching version: {name}@{spec}"))
    }

    pub(super) async fn metadata_versions(&self, name: &str, spec: &str) -> Result<Rc<Vec<Value>>> {
        self.metadata_selected(name, spec, false).await
    }

    async fn metadata_selected(
        &self,
        name: &str,
        spec: &str,
        latest_only: bool,
    ) -> Result<Rc<Vec<Value>>> {
        if !lockfile::valid_name(name) {
            bail!("invalid package name: {name}");
        }
        let (name, spec) = spec::registry_dependency(name, spec)?;
        let key = (name.to_owned(), spec.to_owned(), latest_only);
        if let Some(versions) = self.versions.borrow().get(&key) {
            return Ok(versions.clone());
        }
        if !lockfile::valid_name(name) {
            bail!("invalid package name: {name}");
        }
        let url = self
            .config
            .registry_url(name, self.options.registry.as_deref())?;
        let authorization = self.config.authorization(&url)?;
        let label = format!("{name}@{spec}");
        self.progress.start("resolve", &label);
        let result = async {
            if self.offline() {
                if self.config.authenticated(&url) {
                    bail!("ENOTCACHED: authenticated registry metadata is not cached");
                }
                self.progress
                    .event("resolve", &format!("offline metadata cache {label}"));
                registry::fetch_offline(&url, registry::Query::versions(spec, latest_only))
            } else {
                registry::fetch_using(
                    url.clone(),
                    authorization.as_ref(),
                    registry::Query::versions(spec, latest_only),
                    self.options.no_cache || self.config.authenticated(&url),
                    || self.client(&url, false),
                    Some(&self.progress),
                )
                .await
            }
        }
        .await;
        self.progress.finish(
            "resolve",
            &label,
            if result.is_ok() { "resolved" } else { "failed" },
        );
        let package = result?;
        let versions = Rc::new(package.versions);
        self.versions.borrow_mut().insert(key, versions.clone());
        Ok(versions)
    }

    pub(super) fn omit(&self, data: &Value) -> bool {
        let omit = self
            .options
            .omit
            .as_deref()
            .or_else(|| self.config.get("omit"))
            .unwrap_or("");
        let dev = self.options.production
            || std::env::var("NODE_ENV").as_deref() == Ok("production")
            || omit.split(',').any(|s| s == "dev");
        (dev && flag(data, "dev"))
            || (omit.split(',').any(|s| s == "optional") && flag(data, "optional"))
            || (omit.split(',').any(|s| s == "peer") && flag(data, "peer"))
    }
}
