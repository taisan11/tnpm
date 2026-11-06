//! Shared HTTP/cache path for all registry-facing commands.
use self::{
    cache::{Cache, Document},
    http_cache::Policy,
    transport::Transport,
};
use crate::{
    json::Value,
    profile::Profile,
    semver::{self, Version},
    util::style::progress::Progress,
};
use anyhow::{Context, Result, bail};
use std::{cmp::Ordering, time::SystemTime};

mod cache;
mod http_cache;
pub(crate) mod transport;

pub struct Package {
    pub metadata: Value,
    pub versions: Vec<Value>,
}

#[derive(Clone, Copy)]
pub struct Query<'a> {
    selector: &'a str,
    metadata: bool,
    latest_only: bool,
    fields: &'a [String],
}

impl<'a> Query<'a> {
    pub fn versions(selector: &'a str, latest_only: bool) -> Self {
        Self {
            selector,
            metadata: false,
            latest_only,
            fields: &[],
        }
    }

    pub fn view(selector: &'a str, fields: &'a [String]) -> Self {
        Self {
            selector,
            metadata: true,
            latest_only: false,
            fields,
        }
    }
}

impl<'a> From<&'a str> for Query<'a> {
    fn from(selector: &'a str) -> Self {
        Self {
            selector,
            metadata: true,
            latest_only: false,
            fields: &[],
        }
    }
}

pub fn fetch_offline(url: &reqwest::Url, query: Query<'_>) -> Result<Package> {
    let cache = Cache::home().context("offline registry cache is unavailable")?;
    let document = cache
        .load(url.as_str())?
        .with_context(|| format!("ENOTCACHED: {url}"))?;
    // Explicit offline mode permits stale metadata, but never a network fallback.
    resolve_binary(&document, query)
}

pub async fn fetch_using(
    url: reqwest::Url,
    authorization: Option<&reqwest::header::HeaderValue>,
    query: Query<'_>,
    no_cache: bool,
    client: impl FnOnce() -> Result<reqwest::Client>,
    progress: Option<(&Progress, &str)>,
) -> Result<Package> {
    let mut profile = Profile::new("registry");
    let transport = Transport::current();
    profile.text(
        "build_features",
        "reqwest-defaults,gzip,brotli,deflate,zstd",
    );
    profile.text("transport_config", transport.label());
    profile.text("accept_encoding", transport.accept_encoding());
    // Do not share an authenticated response with an anonymous or different user.
    // Tokens are never persisted or used as non-cryptographic cache identifiers.
    let cache = if no_cache || authorization.is_some() {
        None
    } else {
        Cache::home()
    };
    let mut headers = http_cache::request_headers();
    headers.insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static(transport.accept_encoding()),
    );
    let document = cache
        .as_ref()
        .and_then(|cache| cache.load(url.as_str()).ok().flatten())
        .filter(|document| document.policy().matches(&headers));
    profile.mark("cache_read_ms");
    let mut cached = document.and_then(|document| {
        resolve_binary(&document, query)
            .ok()
            .map(|package| (document, package))
    });
    profile.mark("cache_resolve_ms");
    if cached
        .as_ref()
        .is_some_and(|(document, _)| document.policy().is_fresh(SystemTime::now()))
    {
        profile.text("source", "fresh-cache");
        if let Some((progress, name)) = progress {
            progress.event("resolve", &format!("metadata cache {name}"));
        }
        return Ok(cached.take().expect("fresh cache entry exists").1);
    }
    // A fresh cache hit must not initialize TLS, proxies, or an HTTP pool.
    let client = client()?;
    profile.mark("client_build_ms");
    // One retry is allowed if a 304 contains a different ETag. Do not reuse that body.
    for _ in 0..2 {
        let conditional = cached
            .as_ref()
            .map(|(document, _)| document.policy().conditional_headers())
            .unwrap_or_default();
        let mut request = client
            .get(url.clone())
            .headers(headers.clone())
            .headers(conditional.clone());
        if no_cache {
            request = request.header(reqwest::header::CACHE_CONTROL, "no-cache, no-store");
        }
        if let Some(authorization) = authorization {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        profile.mark("request_build_ms");
        if let Some((progress, name)) = progress {
            progress.event("resolve", &format!("HTTP GET metadata {name}"));
        }
        let request_time = SystemTime::now();
        let response = request
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .context("could not fetch registry metadata")?;
        let response_time = SystemTime::now();
        profile.mark("headers_ms");
        profile.number("status", u64::from(response.status().as_u16()));
        profile.text("http_version", format!("{:?}", response.version()));
        profile.text(
            "content_encoding",
            response
                .headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("identity-or-decoded"),
        );
        if let Some(length) = response.content_length() {
            profile.number("content_length", length);
        }
        if let Some(address) = response.remote_addr() {
            profile.text("remote_addr", address.to_string());
        }
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let Some((document, package)) = cached.take().filter(|_| !conditional.is_empty())
            else {
                bail!("registry returned 304 without a usable conditional cache entry");
            };
            if !document.policy().accepts_not_modified(response.headers()) {
                continue;
            }
            let policy = document.policy().revalidated(
                response.headers(),
                &headers,
                request_time,
                response_time,
            );
            if let Some(cache) = &cache {
                cache_warning(cache.refresh(url.as_str(), &document, &policy));
            }
            profile.mark("cache_refresh_ms");
            profile.text("source", "revalidated-cache");
            if let Some((progress, name)) = progress {
                progress.event("resolve", &format!("metadata revalidated (304) {name}"));
            }
            return Ok(package);
        }
        if response.status() != reqwest::StatusCode::OK {
            bail!("unexpected registry HTTP status: {}", response.status());
        }
        let policy = Policy::new(response.headers(), &headers, request_time, response_time);
        if !policy.can_store()
            && let Some(cache) = &cache
        {
            cache_warning(cache.remove(url.as_str()));
        }
        profile.mark("policy_ms");
        // reqwest expands compressed bodies; nojson still handles JSON parsing.
        let bytes = response.bytes().await?;
        profile.mark("body_ms");
        profile.number("decoded_body_bytes", bytes.len() as u64);
        // Validate the response in place; avoid cloning the entire metadata body.
        let text =
            simdutf8::compat::from_utf8(&bytes).context("registry metadata is not valid UTF-8")?;
        profile.mark("utf8_ms");
        let value = Value::parse(text).context("could not parse registry metadata")?;
        profile.mark("json_parse_ms");
        let package = resolve_json(&value, query)?;
        profile.mark("resolve_ms");
        if policy.can_store()
            && let Some(cache) = &cache
        {
            cache_warning(cache.store(url.as_str(), &value, &policy));
        }
        profile.mark("cache_write_ms");
        drop(value);
        profile.mark("json_tree_drop_ms");
        profile.text("source", "network");
        return Ok(package);
    }
    bail!("registry could not revalidate the metadata cache")
}

fn cache_warning(result: std::io::Result<()>) {
    if let Err(error) = result {
        crate::util::style::progress::message(format_args!(
            "tnpm: could not update registry cache: {error}"
        ));
    }
}
fn select_names<'a>(names: &[&'a str], metadata: &Value, selector: &str) -> Result<Vec<&'a str>> {
    let tagged = metadata.get("dist-tags").and_then(|t| t.str(selector));
    if let Some(exact) =
        tagged.or_else(|| Version::parse(selector).map(|_| selector.trim_start_matches('v')))
    {
        return names
            .iter()
            .find(|name| **name == exact)
            .map(|name| vec![*name])
            .with_context(|| format!("version not found: {selector}"));
    }
    let mut selected = Vec::new();
    for name in names {
        match semver::satisfies(name, selector) {
            Some(true) => selected.push(*name),
            Some(false) => {}
            None => {
                bail!("unknown tag or unsupported version range: {selector}");
            }
        }
    }
    if selected.is_empty() {
        bail!("no matching version: {selector}");
    }
    Ok(selected)
}

fn sort_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut parsed: Vec<_> = names.map(|name| (name, Version::parse(name))).collect();
    // Match the persistent index, including SemVer ties (e.g. build metadata).
    parsed.sort_by_key(|(name, _)| *name);
    parsed.sort_by(|(_, a), (_, b)| match (a, b) {
        (Some(a), Some(b)) => a.compare(b),
        _ => Ordering::Equal,
    });
    parsed.into_iter().map(|(name, _)| name).collect()
}

fn checked_package(metadata: Value, versions: Vec<Value>) -> Result<Package> {
    if versions
        .iter()
        .any(|value| !matches!(value, Value::Object(_)))
    {
        bail!("registry version must contain an object");
    }
    Ok(Package { metadata, versions })
}

fn resolve_json(value: &Value, query: Query<'_>) -> Result<Package> {
    let versions = value
        .get("versions")
        .filter(|v| matches!(v, Value::Object(_)))
        .context("registry response has no versions object")?;
    let names = sort_names(versions.members().iter().map(|(key, _)| key.as_str()));
    let mut selected = select_names(&names, value, query.selector)?;
    if query.latest_only {
        selected = selected.into_iter().rev().take(1).collect();
    }
    let mut metadata = if query.metadata {
        Value::Object(
            value
                .members()
                .iter()
                .filter(|(key, _)| {
                    key != "versions" && (query.fields.is_empty() || query.fields.contains(key))
                })
                .cloned()
                .collect(),
        )
    } else {
        Value::object()
    };
    if query.metadata && (query.fields.is_empty() || query.fields.iter().any(|f| f == "versions")) {
        metadata.set(
            "versions",
            Value::Array(names.into_iter().map(Value::from).collect()),
        );
    }
    let selected = selected
        .into_iter()
        .map(|name| {
            versions
                .get(name)
                .cloned()
                .context("registry version not found")
        })
        .collect::<Result<Vec<_>>>()?;
    checked_package(metadata, selected)
}

fn resolve_binary(document: &Document, query: Query<'_>) -> Result<Package> {
    let root = document.root();
    let versions = root.member("versions")?.context("cache has no versions")?;
    let tagged = root
        .member("dist-tags")?
        .and_then(|tags| tags.member(query.selector).ok().flatten())
        .and_then(|tag| tag.string().ok());
    let exact = tagged
        .or_else(|| Version::parse(query.selector).map(|_| query.selector.trim_start_matches('v')));
    let mut selected = Vec::new();
    if let Some(name) = exact {
        selected.push(
            versions
                .member(name)?
                .with_context(|| format!("version not found: {}", query.selector))?
                .value()?,
        );
    } else {
        let count = document.version_count();
        let early_exit = query.latest_only && document.all_versions_valid();
        for n in 0..count {
            let index = if early_exit { count - 1 - n } else { n };
            let (name, node) = document.version(index)?;
            match semver::satisfies(name, query.selector) {
                Some(true) => {
                    selected.push(node.value()?);
                    if early_exit {
                        break;
                    }
                }
                Some(false) => {}
                None => bail!(
                    "unknown tag or unsupported version range: {}",
                    query.selector
                ),
            }
        }
        if selected.is_empty() {
            bail!("no matching version: {}", query.selector);
        }
        if query.latest_only && !early_exit {
            selected = selected.into_iter().rev().take(1).collect();
        }
    }
    let mut metadata = if !query.metadata {
        Value::object()
    } else if query.fields.is_empty() {
        root.object_except("versions")?
    } else {
        let mut metadata = Value::object();
        for field in query.fields {
            if field != "versions"
                && metadata.get(field).is_none()
                && let Some(value) = root.member(field)?
            {
                metadata.set(field, value.value()?);
            }
        }
        metadata
    };
    if query.metadata && (query.fields.is_empty() || query.fields.iter().any(|f| f == "versions")) {
        let names = (0..document.version_count())
            .map(|i| document.version(i).map(|(name, _)| Value::from(name)))
            .collect::<std::io::Result<Vec<_>>>()?;
        metadata.set("versions", Value::Array(names));
    }
    checked_package(metadata, selected)
}
