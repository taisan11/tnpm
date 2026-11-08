//! Per-command configuration, HTTP clients, and metadata memoization.
use super::flag;
use crate::{
    cli::Options, config::Config, json::Value, lockfile, registry, scripts, spec,
    util::style::progress::Progress,
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
    pub(super) source_root: RefCell<std::path::PathBuf>,
    pub(super) workspaces: RefCell<Vec<crate::workspaces::Workspace>>,
    node_version: RefCell<Option<Option<String>>>,
    pub(super) source_tarballs: RefCell<FxHashMap<String, Vec<u8>>>,
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
            source_root: RefCell::new(std::env::current_dir().unwrap_or_default()),
            workspaces: RefCell::new(Vec::new()),
            node_version: RefCell::new(None),
            source_tarballs: RefCell::new(FxHashMap::default()),
        }
    }

    pub(super) fn offline(&self) -> bool {
        self.options.offline || self.config.enabled("offline")
    }
    pub(super) fn node_version(&self) -> Option<String> {
        if self.node_version.borrow().is_none() {
            *self.node_version.borrow_mut() = Some(super::platform::node_version());
        }
        self.node_version.borrow().as_ref().cloned().flatten()
    }
    pub(super) fn peer_policy(&self) -> lockfile::PeerPolicy {
        let strict = self.options.strict_peer_deps || self.config.enabled("strict-peer-deps");
        let legacy = self.options.legacy_peer_deps.unwrap_or_else(|| {
            if self.options.strict_peer_deps {
                false
            } else if let Some(value) = self.config.get("legacy-peer-deps") {
                value == "true"
            } else {
                false
            }
        });
        lockfile::PeerPolicy {
            legacy,
            strict,
            force: self.options.force || self.config.enabled("force"),
        }
    }
    pub(super) fn warn_peers(&self, lock: &lockfile::Lockfile) {
        for warning in lock.peer_warnings(self.peer_policy()) {
            crate::util::style::progress::message(format_args!(
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
        let native = event == "install" && scripts::implicit_native(root, manifest);
        if manifest.get("scripts").and_then(|s| s.str(event)).is_none() && !native {
            return Ok(());
        }
        let label = format!("{} ({event})", root.display());
        self.progress.start("scripts", &label);
        let result = if native {
            scripts::native_build(root, manifest, self.options.json)
        } else {
            scripts::lifecycle_output(root, manifest, event, self.options.json)
        };
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
        let source_key = (name.to_owned(), spec.to_owned(), latest_only);
        if let Some(versions) = self.versions.borrow().get(&source_key) {
            return Ok(versions.clone());
        }
        if let Some(data) = super::sources::metadata(self, name, spec).await? {
            let data = super::bundles::hydrate(self, data).await?;
            let actual_name = data.str("name").map(str::to_owned);
            let data = Rc::new(vec![data]);
            if let Some(name) = actual_name {
                self.versions
                    .borrow_mut()
                    .insert((name, spec.to_owned(), latest_only), data.clone());
            }
            self.versions.borrow_mut().insert(source_key, data.clone());
            return Ok(data);
        }
        let (name, spec) = spec::registry_dependency(name, spec)?;
        let name = name.as_ref();
        let key = (name.to_owned(), spec.to_owned(), latest_only);
        if let Some(versions) = self.versions.borrow().get(&key) {
            return Ok(versions.clone());
        }
        if !lockfile::valid_name(name) {
            bail!("invalid package name: {name}");
        }
        let url = if name.starts_with("@jsr/") {
            crate::jsr::registry_url(self.config, name)?
        } else {
            self.config
                .registry_url(name, self.options.registry.as_deref())?
        };
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
                    Some((&self.progress, name)),
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
        let mut hydrated = Vec::with_capacity(package.versions.len());
        for data in package.versions {
            hydrated.push(super::bundles::hydrate(self, data).await?);
        }
        let versions = Rc::new(hydrated);
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
