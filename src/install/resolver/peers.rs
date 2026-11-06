//! Joint physical placement of peer sets with version/placement backtracking.
//! Network requests are yielded to the async driver, keeping independent fetches parallel.
use super::{CONCURRENCY, Context, Dependency, Lockfile, Value, descriptor, flag, lockfile, spec};
use anyhow::{Result, bail};
use futures_util::{StreamExt, stream};
use rustc_hash::FxHashMap;
use std::{
    collections::{BTreeSet, VecDeque},
    rc::Rc,
};

type Key = (String, String);
type Catalog = FxHashMap<Key, std::result::Result<Vec<Rc<Value>>, String>>;
const SEARCH_LIMIT: usize = 200_000;
const PREFERRED_SEARCH_LIMIT: usize = 20_000;
const PREFERRED_BACKTRACK_LIMIT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Edge {
    from: String,
    dep: Dependency,
}

#[derive(Clone)]
struct State {
    lock: Lockfile,
    queue: VecDeque<Edge>,
    overridden: BTreeSet<(String, String)>,
}

enum Choice {
    Place(String, Rc<Value>),
    Skip,
    Override,
}
struct Branch {
    before: State,
    edge: Edge,
    choices: Vec<Choice>,
}

enum Step {
    Fetch(Vec<Key>),
    Done(Lockfile),
    Failed(String),
}

struct Search<'a> {
    state: State,
    branches: Vec<Branch>,
    catalog: Catalog,
    manifest: &'a Value,
    old: Option<&'a Lockfile>,
    policy: lockfile::PeerPolicy,
    update: bool,
    targets: &'a [String],
    steps: usize,
    backtracks: usize,
    error: String,
    allow_override: bool,
}

impl State {
    fn enqueue(&mut self, edge: Edge) {
        if !self.queue.contains(&edge) {
            self.queue.push_back(edge);
        }
    }

    fn satisfied(&self, edge: &Edge, catalog: &Catalog) -> bool {
        self.lock
            .resolve_dependency(&edge.from, &edge.dep)
            .is_some_and(|p| {
                let entry = &self.lock.packages[&p];
                accepts(entry, &edge.dep, catalog)
                    && super::super::overrides::same_scope(
                        self.lock.packages.get("").unwrap_or(&Value::Null),
                        self.lock.packages.get(&edge.from),
                        &edge.dep.name,
                        entry,
                    )
                    && (!edge.dep.peer
                        || edge.from.is_empty()
                        || !self
                            .lock
                            .packages
                            .contains_key(&lockfile::child(&edge.from, &edge.dep.name)))
            })
    }
}

fn accepts(entry: &Value, dep: &Dependency, catalog: &Catalog) -> bool {
    spec::satisfies(&dep.name, &dep.spec, entry).unwrap_or_else(|| {
        catalog
            .get(&(dep.name.clone(), dep.spec.clone()))
            .and_then(|r| r.as_ref().ok())
            .is_some_and(|entries| {
                entries.iter().any(|v| {
                    v.str("version") == entry.str("version") && v.str("name") == entry.str("name")
                })
            })
    })
}

fn edges(from: &str, data: &Value) -> Vec<Edge> {
    lockfile::dependencies(data, from.is_empty())
        .into_iter()
        .map(|dep| Edge {
            from: from.into(),
            dep,
        })
        .collect()
}

impl Search<'_> {
    fn retry_with_overrides(&mut self) -> bool {
        if self.allow_override || !self.policy.allows_mismatch() {
            return false;
        }
        let mut lock = Lockfile::default();
        lock.packages
            .insert("".into(), lockfile::root_entry(self.manifest));
        self.state = State {
            lock,
            queue: edges("", self.manifest).into(),
            overridden: BTreeSet::new(),
        };
        self.branches.clear();
        self.steps = 0;
        self.backtracks = 0;
        self.allow_override = true;
        true
    }

    fn placements(&self, edge: &Edge) -> Vec<String> {
        let mut at = edge.from.as_str();
        if edge.dep.peer && !at.is_empty() {
            at = lockfile::parent(at);
        }
        // A peer cannot be placed underneath a member of the same peer set.
        while !at.is_empty()
            && self
                .state
                .lock
                .packages
                .get(at)
                .and_then(|v| v.get("peerDependencies"))
                .and_then(|v| v.get(&edge.dep.name))
                .is_some()
        {
            at = lockfile::parent(at);
        }
        let mut places = Vec::new();
        loop {
            let path = lockfile::child(at, &edge.dep.name);
            if !self.state.lock.packages.contains_key(&path) {
                places.push(path);
            } else {
                break;
            }
            if at.is_empty() {
                break;
            }
            at = lockfile::parent(at);
        }
        places.reverse();
        places
    }

    fn root_accepts(&self, path: &str, entry: &Value) -> bool {
        if !lockfile::parent(path).is_empty() {
            return true;
        }
        // Reserve root declarations even if the corresponding edge is still queued.
        lockfile::dependencies(self.manifest, true)
            .iter()
            .filter(|d| d.name == lockfile::name_at(path))
            .filter(|d| !d.peer || !self.allow_override || !self.policy.allows_mismatch())
            .all(|dep| accepts(entry, dep, &self.catalog))
    }

    fn apply(&mut self, edge: &Edge, choice: Choice) -> Result<()> {
        match choice {
            Choice::Skip => {}
            Choice::Override => {
                self.state
                    .overridden
                    .insert((edge.from.clone(), edge.dep.name.clone()));
            }
            Choice::Place(path, entry) => {
                let mut entry = (*entry).clone();
                super::super::overrides::apply(
                    self.manifest,
                    self.state.lock.packages.get(&edge.from),
                    &edge.dep.name,
                    &mut entry,
                )?;
                if edge.dep.optional && !edge.dep.peer
                    || self
                        .state
                        .lock
                        .packages
                        .get(&edge.from)
                        .is_some_and(|v| flag(v, "optional"))
                {
                    entry.set("optional", Value::Bool(true));
                }
                let outgoing = edges(&path, &entry);
                let bundled = super::super::bundles::insert(
                    &mut self.state.lock,
                    self.manifest,
                    &path,
                    &entry,
                )?;
                self.state.lock.packages.insert(path.clone(), entry);
                // Changing visibility can activate an optional peer or shadow a
                // previously satisfied peer. Recheck affected edges, not just new ones.
                let affected: Vec<_> = self
                    .state
                    .lock
                    .packages
                    .iter()
                    .flat_map(|(p, v)| edges(p, v))
                    .filter(|e| {
                        e.dep.name == edge.dep.name && !self.state.satisfied(e, &self.catalog)
                    })
                    .collect();
                for e in outgoing
                    .into_iter()
                    .chain(bundled.into_iter().map(|(from, dep)| Edge { from, dep }))
                    .chain(affected)
                {
                    self.state.enqueue(e);
                }
            }
        }
        Ok(())
    }

    fn backtrack(&mut self) -> bool {
        while let Some(mut branch) = self.branches.pop() {
            if let Some(choice) = branch.choices.pop() {
                self.backtracks += 1;
                self.state = branch.before.clone();
                if let Err(error) = self.apply(&branch.edge, choice) {
                    self.error = error.to_string();
                    continue;
                }
                if !branch.choices.is_empty() {
                    self.branches.push(branch);
                }
                return true;
            }
        }
        false
    }

    fn advance(&mut self) -> Step {
        loop {
            self.steps += 1;
            let limit = if !self.allow_override && self.policy.allows_mismatch() {
                PREFERRED_SEARCH_LIMIT
            } else {
                SEARCH_LIMIT
            };
            if self.steps > limit
                || (!self.allow_override
                    && self.policy.allows_mismatch()
                    && self.backtracks >= PREFERRED_BACKTRACK_LIMIT)
            {
                return Step::Failed(
                    "ERESOLVE: peer dependency search exceeded 200,000 decisions".into(),
                );
            }
            if self.state.lock.packages.len() > 50_000 {
                return Step::Failed("dependency graph exceeds 50,000 packages".into());
            }
            let Some(edge) = self.state.queue.pop_front() else {
                let policy = if self.allow_override {
                    self.policy
                } else {
                    lockfile::PeerPolicy {
                        strict: true,
                        force: false,
                        ..self.policy
                    }
                };
                match self.state.lock.reachable_with(self.manifest, true, policy) {
                    Ok(used) => {
                        self.state
                            .lock
                            .packages
                            .retain(|p, _| p.is_empty() || used.contains(p));
                        self.state.lock.classify_with(self.manifest, self.policy);
                        return Step::Done(self.state.lock.clone());
                    }
                    Err(error) => {
                        self.error = error.to_string();
                        if self.backtrack() {
                            continue;
                        }
                        return Step::Failed(self.error.clone());
                    }
                }
            };
            if !lockfile::valid_name(&edge.dep.name) {
                return Step::Failed(format!("invalid dependency name: {}", edge.dep.name));
            }
            if edge.dep.peer && self.policy.legacy {
                continue;
            }
            if self
                .state
                .overridden
                .contains(&(edge.from.clone(), edge.dep.name.clone()))
                && edge.dep.peer
            {
                continue;
            }
            if self.state.satisfied(&edge, &self.catalog) {
                continue;
            }
            let found = self.state.lock.resolve_dependency(&edge.from, &edge.dep);
            if edge.dep.peer && edge.dep.optional && found.is_none() {
                continue;
            }
            let key = (edge.dep.name.clone(), edge.dep.spec.clone());
            if !self.catalog.contains_key(&key) {
                self.state.queue.push_front(edge);
                let requests = self
                    .state
                    .queue
                    .iter()
                    .filter(|e| {
                        !(e.dep.peer
                            && e.dep.optional
                            && self
                                .state
                                .lock
                                .resolve_dependency(&e.from, &e.dep)
                                .is_none())
                    })
                    .map(|e| (e.dep.name.clone(), e.dep.spec.clone()))
                    .filter(|k| !self.catalog.contains_key(k))
                    .collect::<BTreeSet<_>>();
                return Step::Fetch(requests.into_iter().take(CONCURRENCY).collect());
            }
            let mut entries = self
                .catalog
                .get(&key)
                .and_then(|v| v.as_ref().ok())
                .cloned()
                .unwrap_or_default();
            let refresh = self.update
                && (self.targets.is_empty()
                    || self
                        .targets
                        .iter()
                        .any(|s| spec::package_spec(s).is_ok_and(|(n, _)| n == edge.dep.name)));
            if !refresh
                && self.manifest.get("overrides").is_none()
                && let Some(old) = self.old
                && let Some(path) = old.resolve_dependency(&edge.from, &edge.dep)
            {
                let entry = &old.packages[&path];
                if accepts(entry, &edge.dep, &self.catalog) {
                    entries.retain(|v| v.str("version") != entry.str("version"));
                    entries.insert(0, Rc::new(entry.clone()));
                }
            }
            let mut choices = Vec::new();
            let locations = self.placements(&edge);
            for entry in entries {
                for path in &locations {
                    if path.matches("node_modules/").count() > 128 {
                        continue;
                    }
                    if self.root_accepts(path, &entry) {
                        choices.push(Choice::Place(path.clone(), entry.clone()));
                    }
                }
            }
            let optional = !edge.dep.peer
                && (edge.dep.optional
                    || self
                        .state
                        .lock
                        .packages
                        .get(&edge.from)
                        .is_some_and(|v| flag(v, "optional")));
            if optional {
                choices.push(Choice::Skip);
            }
            if edge.dep.peer
                && self.allow_override
                && found.is_some()
                && self.policy.allows_mismatch()
            {
                // The strict pass already searched for a compatible placement.
                // Prefer the visible peer in the fallback rather than repeating
                // the same potentially enormous search before trying an override.
                choices.insert(0, Choice::Override);
            }
            self.error = if edge.dep.peer {
                lockfile::peer_error(
                    &edge.from,
                    &edge.dep,
                    found.as_ref().map(|p| &self.state.lock.packages[p]),
                )
            } else {
                format!(
                    "ERESOLVE: cannot place {}@{} required by {}{}",
                    edge.dep.name,
                    edge.dep.spec,
                    edge.from,
                    self.catalog
                        .get(&key)
                        .and_then(|r| r.as_ref().err())
                        .map(|e| format!(": {e}"))
                        .unwrap_or_default()
                )
            };
            if choices.is_empty() {
                if self.backtrack() {
                    continue;
                }
                return Step::Failed(self.error.clone());
            }
            choices.reverse();
            let choice = choices.pop().unwrap();
            let before = if choices.is_empty() {
                None
            } else {
                Some(self.state.clone())
            };
            if let Err(error) = self.apply(&edge, choice) {
                self.error = error.to_string();
                if self.backtrack() {
                    continue;
                }
                return Step::Failed(self.error.clone());
            }
            if let Some(before) = before {
                self.branches.push(Branch {
                    before,
                    edge,
                    choices,
                });
            }
        }
    }
}

pub(super) async fn resolve(
    ctx: &Context<'_>,
    manifest: &Value,
    old: Option<&Lockfile>,
    update: bool,
) -> Result<Lockfile> {
    let mut lock = Lockfile::default();
    lock.packages
        .insert("".into(), lockfile::root_entry(manifest));
    let mut search = Search {
        state: State {
            lock,
            queue: edges("", manifest).into(),
            overridden: BTreeSet::new(),
        },
        branches: Vec::new(),
        catalog: FxHashMap::default(),
        manifest,
        old,
        policy: ctx.peer_policy(),
        update,
        targets: &ctx.options.arguments,
        steps: 0,
        backtracks: 0,
        error: "ERESOLVE: unable to resolve peer dependencies".into(),
        allow_override: false,
    };
    loop {
        match search.advance() {
            Step::Done(lock) => return Ok(lock),
            Step::Failed(error) => {
                // Prefer a valid peer set, then allow mismatched existing versions
                // with warnings unless strict peer validation was requested.
                if search.retry_with_overrides() {
                    continue;
                }
                bail!("{error}");
            }
            Step::Fetch(requests) => {
                let responses: Vec<_> =
                    stream::iter(requests.into_iter().map(|(name, spec)| async move {
                        let result = ctx
                            .metadata_versions(&name, &spec)
                            .await
                            .and_then(|versions| {
                                versions
                                    .iter()
                                    .rev()
                                    .map(descriptor)
                                    .map(|r| r.map(Rc::new))
                                    .collect::<Result<Vec<_>>>()
                            })
                            .map_err(|e| format!("{e:#}"));
                        ((name, spec), result)
                    }))
                    .buffer_unordered(CONCURRENCY)
                    .collect()
                    .await;
                search.catalog.extend(responses);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(manifest: &Value, policy: lockfile::PeerPolicy) -> Search<'_> {
        let mut lock = Lockfile::default();
        lock.packages
            .insert("".into(), lockfile::root_entry(manifest));
        Search {
            state: State {
                lock,
                queue: edges("", manifest).into(),
                overridden: BTreeSet::new(),
            },
            branches: Vec::new(),
            catalog: FxHashMap::default(),
            manifest,
            old: None,
            policy,
            update: false,
            targets: &[],
            steps: 0,
            backtracks: 0,
            error: String::new(),
            allow_override: false,
        }
    }

    #[test]
    fn exhausted_preferred_search_retries_with_a_fresh_budget() {
        let manifest = Value::parse(r#"{"dependencies":{"host":"1"}}"#).unwrap();
        let mut search = search(&manifest, lockfile::PeerPolicy::default());
        search.steps = PREFERRED_SEARCH_LIMIT;
        assert!(matches!(search.advance(), Step::Failed(_)));
        assert!(search.retry_with_overrides());
        assert_eq!(search.steps, 0);
        assert!(matches!(search.advance(), Step::Fetch(_)));
        assert!(!search.retry_with_overrides());
    }

    #[test]
    fn strict_search_does_not_fall_back() {
        let manifest = Value::object();
        let mut search = search(
            &manifest,
            lockfile::PeerPolicy {
                strict: true,
                ..Default::default()
            },
        );
        search.steps = SEARCH_LIMIT;
        assert!(matches!(search.advance(), Step::Failed(_)));
        assert!(!search.retry_with_overrides());
    }

    #[test]
    fn preferred_search_limits_backtracking_even_with_steps_remaining() {
        let manifest = Value::parse(r#"{"dependencies":{"host":"1"}}"#).unwrap();
        let mut search = search(&manifest, lockfile::PeerPolicy::default());
        search.backtracks = PREFERRED_BACKTRACK_LIMIT;
        assert!(matches!(search.advance(), Step::Failed(_)));
        assert!(search.retry_with_overrides());
        assert_eq!(search.backtracks, 0);
        assert!(matches!(search.advance(), Step::Fetch(_)));
    }
}
