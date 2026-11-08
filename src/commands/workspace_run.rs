//! Bounded workspace execution, optionally in dependency-first waves.
use crate::{cli::Options, config::Config, lockfile, semver, workspaces::Workspace};
use anyhow::{Context, Result, bail};
use std::{collections::BTreeSet, path::Path};

pub(crate) fn run(
    root: &Path,
    selected: &[&Workspace],
    options: &Options,
    config: &Config,
) -> Result<()> {
    // Plan the complete DAG before starting any scripts, including cycle detection.
    let mut pending: BTreeSet<_> = (0..selected.len()).collect();
    let mut dependencies = vec![BTreeSet::new(); selected.len()];
    if options.topological {
        for (i, workspace) in selected.iter().enumerate() {
            for dep in lockfile::dependencies(&workspace.manifest, true)
                .into_iter()
                .filter(|d| !d.peer)
            {
                if let Some((j, _)) = selected.iter().enumerate().find(|(_, w)| {
                    let range = dep.spec.strip_prefix("workspace:").unwrap_or(&dep.spec);
                    w.name == dep.name
                        && ((dep.spec.starts_with("workspace:")
                            && matches!(range, "*" | "^" | "~"))
                            || semver::satisfies(
                                w.manifest.str("version").unwrap_or("0.0.0"),
                                range,
                            ) == Some(true))
                }) {
                    dependencies[i].insert(j);
                }
            }
        }
    }
    let mut waves = Vec::new();
    while !pending.is_empty() {
        let ready: Vec<_> = pending
            .iter()
            .copied()
            .filter(|i| dependencies[*i].is_disjoint(&pending))
            .collect();
        if ready.is_empty() {
            bail!(
                "workspace dependency cycle: {}",
                pending
                    .iter()
                    .map(|i| selected[*i].name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        for i in &ready {
            pending.remove(i);
        }
        waves.push(ready);
    }
    let concurrency = if options.parallel {
        options
            .concurrency
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, usize::from))
    } else {
        1
    };
    for wave in waves {
        for batch in wave.chunks(concurrency) {
            // All started scripts are joined even if one fails. No dependent wave starts on failure.
            let results = std::thread::scope(|scope| {
                let jobs: Vec<_> = batch
                    .iter()
                    .map(|i| {
                        let workspace = selected[*i];
                        scope.spawn(move || {
                            super::run::run(&root.join(&workspace.relative), options, config)
                                .with_context(|| format!("workspace {} failed", workspace.name))
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .map(|job| {
                        job.join()
                            .unwrap_or_else(|_| Err(anyhow::anyhow!("workspace runner panicked")))
                    })
                    .collect::<Vec<_>>()
            });
            for result in results {
                result?;
            }
        }
    }
    Ok(())
}
