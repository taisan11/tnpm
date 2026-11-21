use super::{Context, Value, descriptor};
use crate::install::store;
use crate::{lockfile, spec};
use anyhow::Result;
use futures_util::{FutureExt, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeSet, VecDeque};

type Key = (String, String);
type Response = (Key, String, Result<Value>);

pub(super) struct Pipeline<'c, 'o> {
    ctx: &'c Context<'o>,
    queued: VecDeque<Key>,
    scheduled: FxHashSet<Key>,
    ready: FxHashMap<Key, Value>,
    busy: FxHashSet<String>,
    fetching: FuturesUnordered<LocalBoxFuture<'c, Response>>,
    pub(super) errors: FxHashMap<Key, anyhow::Error>,
    discover: bool,
    packages: VecDeque<(Value, String)>,
    package_keys: FxHashSet<String>,
    preparing: FuturesUnordered<LocalBoxFuture<'c, ()>>,
}

impl<'c, 'o> Pipeline<'c, 'o> {
    pub(super) fn new(ctx: &'c Context<'o>, discover: bool) -> Self {
        Self {
            ctx,
            queued: VecDeque::new(),
            scheduled: FxHashSet::default(),
            ready: FxHashMap::default(),
            busy: FxHashSet::default(),
            fetching: FuturesUnordered::new(),
            errors: FxHashMap::default(),
            discover,
            packages: VecDeque::new(),
            package_keys: FxHashSet::default(),
            preparing: FuturesUnordered::new(),
        }
    }

    pub(super) fn prefetch(&mut self, entry: &Value, name: &str) {
        if !self.discover
            || self.ctx.config.get("node-linker") == Some("isolated")
            || self.ctx.offline()
            || self.ctx.options.lock_only
            || self.ctx.options.dry_run
            || !matches!(
                self.ctx.options.command.as_str(),
                "install" | "update" | "ci"
            )
            || entry.get("tnpmSpec").is_some()
            || entry.get("tnpmBundled").is_some()
            || entry.get("tnpmBundlePath").is_some()
            || entry.get("bundleDependencies").is_some()
            || entry.get("bundledDependencies").is_some()
            || !crate::install::platform::compatible(entry)
        {
            return;
        }
        let Some(cache) =
            store::package_cache_path(self.ctx, entry, &format!("node_modules/{name}"))
        else {
            return;
        };
        if store::cached_package(&cache, entry.str("integrity")).is_some() {
            return;
        }
        let key = format!(
            "{}\n{}",
            entry.str("resolved").unwrap_or(""),
            entry.str("integrity").unwrap_or("")
        );
        if self.package_keys.insert(key) {
            self.packages.push_back((entry.clone(), name.to_owned()));
        }
    }

    fn start_preparing(&mut self) {
        while self.preparing.len() < 8 {
            let Some((entry, name)) = self.packages.pop_front() else {
                break;
            };
            let ctx = self.ctx;
            self.preparing.push(
                async move {
                    let result = async {
                        let cache =
                            store::package_cache_path(ctx, &entry, &format!("node_modules/{name}"))
                                .unwrap();
                        let bytes = store::tarball(ctx, &entry, &name).await?;
                        ctx.progress
                            .event("prepare", &format!("extract node_modules/{name}"));
                        let integrity = entry.str("integrity").unwrap().to_owned();
                        tokio::task::spawn_blocking(move || -> Result<()> {
                            let parent = cache.parent().unwrap();
                            std::fs::create_dir_all(parent)?;
                            let temporary = tempfile::Builder::new()
                                .prefix(".prefetch-")
                                .tempdir_in(parent)?;
                            let payload = temporary.path().join("package");
                            std::fs::create_dir(&payload)?;
                            store::unpack(&bytes, &payload)?;
                            store::publish_package_cache(temporary.path(), &cache, Some(&integrity))
                        })
                        .await??;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await;
                    if let Err(error) = result {
                        ctx.progress.event(
                            "prepare",
                            &format!("deferred package prefetch {name}: {error:#}"),
                        );
                    }
                }
                .boxed_local(),
            );
        }
    }

    pub(super) async fn finish(&mut self) {
        loop {
            self.start_preparing();
            if self.preparing.next().await.is_none() {
                break;
            }
        }
    }

    fn enqueue(&mut self, key: Key, memo: &FxHashMap<Key, Value>) {
        if !memo.contains_key(&key) && self.scheduled.insert(key.clone()) {
            self.queued.push_back(key);
        }
    }

    pub(super) async fn resolve(
        &mut self,
        requests: BTreeSet<Key>,
        memo: &mut FxHashMap<Key, Value>,
    ) {
        for key in &requests {
            if let Some(entry) = self.ready.remove(key)
                && !memo.contains_key(key)
            {
                self.prefetch(&entry, &key.0);
                memo.insert(key.clone(), entry);
            }
            self.enqueue(key.clone(), memo);
        }
        while requests
            .iter()
            .any(|key| !memo.contains_key(key) && !self.errors.contains_key(key))
        {
            self.start_preparing();
            for _ in 0..self.queued.len() {
                if self.fetching.len() >= 32 {
                    break;
                }
                let key = self.queued.pop_front().unwrap();
                let document = spec::registry_dependency(&key.0, &key.1)
                    .map(|(name, _)| name.into_owned())
                    .unwrap_or_else(|_| format!("{}@{}", key.0, key.1));
                if !self.busy.insert(document.clone()) {
                    self.queued.push_back(key);
                    continue;
                }
                let ctx = self.ctx;
                self.fetching.push(
                    async move {
                        let result = ctx
                            .metadata(&key.0, &key.1)
                            .await
                            .and_then(|v| descriptor(&v));
                        (key, document, result)
                    }
                    .boxed_local(),
                );
            }
            let response = if self.preparing.is_empty() {
                self.fetching.next().await
            } else {
                match futures_util::future::select(self.fetching.next(), self.preparing.next())
                    .await
                {
                    futures_util::future::Either::Left((response, _)) => response,
                    futures_util::future::Either::Right(_) => continue,
                }
            };
            let Some((key, document, result)) = response else {
                break;
            };
            self.busy.remove(&document);
            match result {
                Ok(entry) => {
                    if self.discover
                        && self.scheduled.len() < 50_000
                        && entry.get("tnpmBundled").is_none()
                        && entry.get("tnpmSpec").is_none()
                    {
                        for dep in lockfile::dependencies(&entry, false)
                            .into_iter()
                            .filter(|d| !d.peer)
                        {
                            if spec::registry_dependency(&dep.name, &dep.spec).is_ok() {
                                self.enqueue((dep.name, dep.spec), memo);
                            }
                        }
                    }
                    if requests.contains(&key) {
                        self.prefetch(&entry, &key.0);
                        memo.entry(key).or_insert(entry);
                    } else {
                        self.ready.insert(key, entry);
                    }
                }
                Err(error) => {
                    self.errors.insert(key, error);
                }
            }
        }
    }
}
