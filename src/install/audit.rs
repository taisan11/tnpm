//! Explicit Bulk Advisory audit. Never run implicitly during install or modify dependencies.
use super::{context::Context, sources};
use crate::{
    json::Value,
    lockfile::{self, Lockfile},
    manifest, semver,
};
use anyhow::{Context as _, Result, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const SEVERITIES: &[&str] = &["info", "low", "moderate", "high", "critical", "none"];
type Versions = BTreeMap<String, Vec<String>>;
type Packages = BTreeMap<String, Versions>;

pub(super) async fn run(ctx: &Context<'_>, root: &Path) -> Result<()> {
    let options = ctx.options;
    ensure!(
        !ctx.offline(),
        "audit requires network access; --offline is not supported"
    );
    let level = options
        .audit_level
        .as_deref()
        .or_else(|| ctx.config.get("audit-level"))
        .unwrap_or("info");
    let threshold = SEVERITIES
        .iter()
        .position(|s| *s == level)
        .context("invalid audit-level; use info, low, moderate, high, critical, or none")?;
    let project = manifest::read(root)?.context("audit requires package.json")?;
    let project = sources::effective(root, &project)?.0;
    let mut lock =
        Lockfile::read(root)?.context("audit requires tnpm-lock.json; run install first")?;
    lock.validate_with(&project, ctx.peer_policy())?;
    lock.classify_with(&project, ctx.peer_policy());
    let mut selected: BTreeSet<_> = lock
        .packages
        .iter()
        .filter(|(p, e)| !p.is_empty() && !ctx.omit(e))
        .map(|(p, _)| p.clone())
        .collect();
    let reachable = lock.reachable_selected(&project, &selected)?;
    selected.retain(|p| reachable.contains(p));
    // Send each package only to its configured registry; private scope names never leak to the default registry.
    let mut groups: BTreeMap<String, Packages> = BTreeMap::new();
    let mut skipped = 0;
    for path in selected {
        let entry = &lock.packages[&path];
        if entry.get("tnpmSpec").is_some() || entry.get("link") == Some(&Value::Bool(true)) {
            skipped += 1;
            continue;
        }
        let Some(version) = entry
            .str("version")
            .filter(|v| semver::Version::parse(v).is_some())
        else {
            skipped += 1;
            continue;
        };
        let name = entry
            .str("name")
            .unwrap_or_else(|| lockfile::name_at(&path));
        let mut endpoint = ctx.config.registry_url(name, options.registry.as_deref())?;
        endpoint
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid audit registry URL"))?
            .pop()
            .extend(["-", "npm", "v1", "security", "advisories", "bulk"]);
        groups
            .entry(endpoint.to_string())
            .or_default()
            .entry(name.to_owned())
            .or_default()
            .entry(version.to_owned())
            .or_default()
            .push(path);
    }
    let mut findings = Vec::new();
    let mut counts = [0usize; 5];
    let mut failed = false;
    for (endpoint, packages) in groups {
        let url = reqwest::Url::parse(&endpoint)?;
        let payload = Value::Object(
            packages
                .iter()
                .map(|(name, versions)| {
                    (
                        name.clone(),
                        Value::Array(versions.keys().map(|v| Value::from(v.as_str())).collect()),
                    )
                })
                .collect(),
        );
        let mut request = ctx
            .config
            .audit_client(&url)?
            .post(url.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json")
            .body(payload.pretty());
        if let Some(auth) = ctx.config.authorization(&url)? {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let response = request
            .send()
            .await
            .context("audit request failed")?
            .error_for_status()
            .context("audit registry rejected Bulk Advisory request")?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "unexpected audit HTTP status: {}",
            response.status()
        );
        let response =
            Value::parse(&response.text().await?).context("invalid audit response JSON")?;
        ensure!(
            matches!(response, Value::Object(_)),
            "audit response must be an object"
        );
        for (name, advisories) in response.members() {
            let Some(versions) = packages.get(name) else {
                bail!("audit response contains an unrequested package: {name}");
            };
            let Value::Array(advisories) = advisories else {
                bail!("invalid audit advisory list for {name}");
            };
            for advisory in advisories {
                let severity = advisory
                    .str("severity")
                    .context("audit advisory has no severity")?;
                let rank = SEVERITIES[..5]
                    .iter()
                    .position(|s| *s == severity)
                    .context("invalid audit advisory severity")?;
                let range = advisory
                    .str("vulnerable_versions")
                    .context("audit advisory has no vulnerable_versions")?;
                ensure!(
                    advisory.str("title").is_some()
                        && advisory.str("url").is_some()
                        && advisory.get("id").is_some()
                        && advisory.str("name").is_none_or(|n| n == name),
                    "invalid audit advisory for {name}"
                );
                let mut affected = Vec::new();
                for (version, paths) in versions {
                    let vulnerable = semver::satisfies(version, range)
                        .context("unsupported audit vulnerable_versions range")?;
                    if !vulnerable {
                        continue;
                    }
                    for path in paths {
                        let mut node = Value::object();
                        node.set("version", Value::from(version.as_str()));
                        node.set("location", Value::from(path.as_str()));
                        affected.push(node);
                    }
                }
                if affected.is_empty() {
                    continue;
                }
                counts[rank] += 1;
                failed |= rank >= threshold;
                let mut finding = advisory.clone();
                finding.set("name", Value::from(name.as_str()));
                finding.set("affected", Value::Array(affected));
                findings.push(finding);
            }
        }
    }
    let total = findings.len();
    let mut report = Value::object();
    report.set("advisories", Value::Array(findings.clone()));
    let mut summary = Value::object();
    for (severity, count) in SEVERITIES.iter().zip(counts) {
        summary.set(severity, Value::Number(count.to_string()));
    }
    summary.set("total", Value::Number(total.to_string()));
    report.set("vulnerabilities", summary);
    report.set("skipped", Value::Number(skipped.to_string()));
    if options.json {
        println!("{}", report.pretty());
    } else {
        for finding in findings {
            println!(
                "{}: {} - {}\n  {}",
                finding.str("severity").unwrap(),
                finding.str("name").unwrap(),
                finding.str("title").unwrap(),
                finding.str("url").unwrap()
            );
            if let Some(Value::Array(nodes)) = finding.get("affected") {
                for node in nodes {
                    println!(
                        "  {} ({})",
                        node.str("location").unwrap(),
                        node.str("version").unwrap()
                    );
                }
            }
        }
        println!(
            "{total} vulnerabilities found ({skipped} non-registry/unversioned packages skipped)"
        );
    }
    if failed {
        bail!("vulnerabilities found at or above audit-level {level}");
    }
    Ok(())
}
