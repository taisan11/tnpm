//! JSR package routing. Node installations use the official JSR compatibility artifacts,
//! but the JSR registry and specifier handling remain separate from npm registry options.
use crate::{config::Config, semver, spec};
use anyhow::{Context, Result, ensure};

pub struct Specifier<'a> {
    pub name: &'a str,
    pub range: &'a str,
    pub subpath: &'a str,
}

impl<'a> Specifier<'a> {
    pub fn parse(input: &'a str) -> Result<Self> {
        let text = input
            .strip_prefix("jsr:")
            .context("expected a jsr: specifier")?;
        let scope_end = text.find('/').context("JSR packages require @scope/name")?;
        let end = text[scope_end + 1..]
            .find('/')
            .map_or(text.len(), |i| scope_end + 1 + i);
        let (name, range) = spec::package_spec(&text[..end])?;
        ensure!(!text[..end].ends_with('@'), "empty JSR version selector");
        let (scope, package) = name
            .strip_prefix('@')
            .and_then(|v| v.split_once('/'))
            .context("JSR packages require @scope/name")?;
        ensure!(
            [scope, package].iter().all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')),
            "invalid JSR package name: {name}"
        );
        let range = range.unwrap_or("latest");
        ensure!(
            range == "latest" || semver::satisfies("0.0.0", range).is_some(),
            "invalid JSR version range: {range}"
        );
        let subpath = text.get(end + 1..).unwrap_or("");
        ensure!(
            end == text.len() || !subpath.is_empty(),
            "empty JSR export subpath"
        );
        validate_subpath(subpath)?;
        Ok(Self {
            name,
            range,
            subpath,
        })
    }

    pub fn npm_name(&self) -> String {
        format!(
            "@jsr/{}",
            self.name.trim_start_matches('@').replace('/', "__")
        )
    }
}

/// Translate only the artifact identity; never rewrite the user's jsr: dependency.
pub fn dependency_target(selector: &str) -> Result<(String, &str)> {
    let jsr = Specifier::parse(selector)?;
    ensure!(
        jsr.subpath.is_empty(),
        "JSR dependencies cannot select an export subpath"
    );
    Ok((jsr.npm_name(), jsr.range))
}

/// The @jsr scope always uses JSR's registry configuration. A CLI --registry
/// override for ordinary npm packages must not redirect JSR requests to npm.
pub fn registry_url(config: &Config, name: &str) -> Result<reqwest::Url> {
    ensure!(
        name.starts_with("@jsr/"),
        "expected a JSR npm compatibility package name"
    );
    config.registry_url(name, None)
}

pub fn validate_subpath(path: &str) -> Result<()> {
    ensure!(
        path.is_empty()
            || path.split('/').all(|part| !part.is_empty()
                && part != "."
                && part != ".."
                && !part
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || "\\%?#:".contains(c))),
        "unsafe package export subpath: {path}"
    );
    Ok(())
}
