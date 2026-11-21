//! Shared HTTP/cache path for all registry-facing commands.
use self::{
    cache::{Cache, Document},
    http_cache::Policy,
    transport::Transport,
};
use crate::{
    json::Value,
    profile::Profile,
    semver::{Range, Version},
    util::style::progress::Progress,
};
use anyhow::{Context, Result, bail};
use rustc_hash::FxHashMap;
use std::{
    borrow::Cow,
    cell::RefCell,
    cmp::Ordering,
    collections::VecDeque,
    rc::Rc,
    time::{Duration, SystemTime},
};

mod cache;
mod http_cache;
pub(crate) mod transport;

const MAX_BODY: usize = 128 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 64;
const MAX_DOCUMENT_BYTES: usize = 64 * 1024 * 1024;

thread_local! {
    /// Parsed packuments survive across selectors and resolution waves so one
    /// URL is read/parsed once per command. Entries are bounded by count/size.
    static DOCUMENTS: RefCell<DocumentCache> = RefCell::new(DocumentCache::default());
}

#[derive(Default)]
struct DocumentCache {
    entries: FxHashMap<String, Rc<Document>>,
    order: VecDeque<String>,
    bytes: usize,
}

impl DocumentCache {
    fn get(&self, url: &str) -> Option<Rc<Document>> {
        self.entries.get(url).cloned()
    }

    fn insert(&mut self, url: String, document: Rc<Document>) {
        if self.entries.contains_key(&url) {
            return;
        }
        let size = document.byte_len();
        if size > MAX_DOCUMENT_BYTES {
            return;
        }
        while !self.order.is_empty()
            && (self.bytes.saturating_add(size) > MAX_DOCUMENT_BYTES
                || self.entries.len() >= MAX_DOCUMENTS)
        {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&old) {
                self.bytes = self.bytes.saturating_sub(removed.byte_len());
            }
        }
        self.bytes = self.bytes.saturating_add(size);
        self.order.push_back(url.clone());
        self.entries.insert(url, document);
    }
}

fn remember(key: &str, document: Rc<Document>) {
    DOCUMENTS.with(|documents| documents.borrow_mut().insert(key.to_owned(), document));
}

async fn load_cached(cache: Option<&Cache>, key: &str) -> Result<Option<Rc<Document>>> {
    if let Some(document) = DOCUMENTS.with(|documents| documents.borrow().get(key)) {
        return Ok(Some(document));
    }
    let Some(cache) = cache else {
        return Ok(None);
    };
    let cache = cache.clone();
    let cache_owned = cache.clone();
    let cache_key = key.to_owned();
    match tokio::task::spawn_blocking(move || cache_owned.load(&cache_key)).await {
        Ok(Ok(Some(document))) => {
            let document = Rc::new(document);
            remember(key, document.clone());
            Ok(Some(document))
        }
        Ok(Ok(None)) => Ok(None),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidData => {
            let cache = cache.clone();
            let cache_key = key.to_owned();
            let _ = tokio::task::spawn_blocking(move || cache.remove(&cache_key)).await;
            crate::util::style::progress::message(format_args!(
                "tnpm: warning: removed corrupt registry cache entry for {key}"
            ));
            Ok(None)
        }
        Ok(Err(error)) => {
            crate::util::style::progress::message(format_args!(
                "tnpm: warning: could not read registry cache for {key}: {error}"
            ));
            Ok(None)
        }
        Err(error) => Err(error).context("registry cache task failed"),
    }
}

async fn response_body(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        bail!("registry response exceeds {MAX_BODY} bytes");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("could not read registry response body")?
    {
        if body.len().saturating_add(chunk.len()) > MAX_BODY {
            bail!("registry response exceeds {MAX_BODY} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Retry idempotent GETs on transient transport failures and 429/5xx responses.
async fn send_with_retry(request: &reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        let response = request
            .try_clone()
            .context("registry request cannot be retried")?
            .send()
            .await;
        match response {
            Ok(response)
                if matches!(
                    response.status().as_u16(),
                    408 | 425 | 429 | 500 | 502 | 503 | 504
                ) && attempt < 2 =>
            {
                tokio::time::sleep(Duration::from_millis(100 * (1u64 << attempt))).await;
                attempt += 1;
            }
            Ok(response) => return Ok(response),
            Err(error) if (error.is_connect() || error.is_timeout()) && attempt < 2 => {
                tokio::time::sleep(Duration::from_millis(100 * (1u64 << attempt))).await;
                attempt += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

pub fn fetch_offline(url: &reqwest::Url, query: Query<'_>) -> Result<Package> {
    let key = metadata_key(url, query);
    let mut error = None;
    for key in std::iter::once(key.as_str()).chain((!query.metadata).then_some(url.as_str())) {
        let document = match DOCUMENTS.with(|documents| documents.borrow().get(key)) {
            Some(document) => Some(document),
            None => Cache::home()
                .context("offline registry cache is unavailable")?
                .load(key)?
                .map(Rc::new),
        };
        let Some(document) = document else { continue };
        remember(key, document.clone());
        // Explicit offline mode permits stale metadata, but never a network fallback.
        match resolve_binary(&document, query) {
            Ok(package) => return Ok(package),
            Err(failure) => error = Some(failure),
        }
    }
    if let Some(error) = error {
        return Err(error);
    }
    bail!("ENOTCACHED: {url}")
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
    if !query.metadata {
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.npm.install-v1+json"),
        );
    }
    headers.insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static(transport.accept_encoding()),
    );
    let key = metadata_key(&url, query);
    let document = load_cached(cache.as_ref(), &key)
        .await?
        .filter(|document| document.policy().matches(&headers));
    profile.mark("cache_read_ms");
    // A cache entry can be fresh yet not contain a version the selector needs;
    // only a successful resolution may short-circuit the network.
    let mut cached = document
        .as_ref()
        .and_then(|document| resolve_binary(document, query).ok());
    if document
        .as_ref()
        .is_some_and(|document| document.policy().is_fresh(SystemTime::now()))
        && let Some(package) = cached.take()
    {
        profile.mark("cache_resolve_ms");
        profile.text("source", "fresh-cache");
        if let Some((progress, name)) = progress {
            progress.event("resolve", &format!("metadata cache {name}"));
        }
        return Ok(package);
    }
    profile.mark("cache_resolve_ms");
    // A fresh cache hit must not initialize TLS, proxies, or an HTTP pool.
    let client = client()?;
    profile.mark("client_build_ms");
    // One retry is allowed if a 304 contains a different ETag. Do not reuse that body.
    for _ in 0..2 {
        let conditional = if cached.is_some() {
            document
                .as_ref()
                .map(|document| document.policy().conditional_headers())
                .unwrap_or_default()
        } else {
            reqwest::header::HeaderMap::new()
        };
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
        let response = send_with_retry(&request)
            .await?
            .error_for_status()
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
            let Some(package) = cached.take() else {
                bail!("registry returned 304 without a usable conditional cache entry");
            };
            let Some(document) = document.as_ref() else {
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
                match cache.refresh(&key, document, &policy) {
                    Ok(bytes) if !bytes.is_empty() => {
                        if let Ok(Some(refreshed)) = Document::open(bytes, &key) {
                            remember(&key, Rc::new(refreshed));
                        }
                    }
                    Ok(_) => {}
                    Err(error) => cache_warning(Err(error)),
                }
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
            cache_warning(cache.remove(&key));
        }
        profile.mark("policy_ms");
        // reqwest expands compressed bodies; nojson still handles JSON parsing.
        let body = response_body(response).await?;
        profile.mark("body_ms");
        profile.number("decoded_body_bytes", body.len() as u64);
        // Validate the response in place; avoid cloning the entire metadata body.
        let text =
            simdutf8::compat::from_utf8(&body).context("registry metadata is not valid UTF-8")?;
        profile.mark("utf8_ms");
        let value = Value::parse(text).context("could not parse registry metadata")?;
        profile.mark("json_parse_ms");
        if policy.can_store()
            && let Ok(encoded) = cache::encode(&key, &value, &policy)
            && let Ok(Some(encoded)) = Document::open(encoded, &key)
        {
            // Reuse the exact bytes that are written to disk, including the
            // pre-sorted version index, instead of sorting plain JSON again.
            let encoded = Rc::new(encoded);
            let package = resolve_binary(&encoded, query)?;
            profile.mark("resolve_ms");
            if let Some(cache) = &cache {
                let cache = cache.clone();
                let cache_key = key.clone();
                let bytes = encoded.as_bytes().to_vec();
                let _ = tokio::task::spawn_blocking(move || cache.store_bytes(&cache_key, &bytes))
                    .await;
            }
            remember(&key, encoded.clone());
            profile.mark("cache_write_ms");
            profile.text("source", "network");
            drop(value);
            profile.mark("json_tree_drop_ms");
            return Ok(package);
        }
        let package = resolve_json(&value, query)?;
        profile.mark("resolve_ms");
        if policy.can_store()
            && let Some(cache) = &cache
        {
            cache_warning(cache.store(&key, &value, &policy));
        }
        profile.mark("cache_write_ms");
        drop(value);
        profile.mark("json_tree_drop_ms");
        profile.text("source", "network");
        return Ok(package);
    }
    bail!("registry could not revalidate the metadata cache")
}

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

fn metadata_key(url: &reqwest::Url, query: Query<'_>) -> String {
    if query.metadata {
        url.as_str().to_owned()
    } else {
        format!("{url}#install-v1")
    }
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

fn cache_warning(result: std::io::Result<()>) {
    if let Err(error) = result {
        crate::util::style::progress::message(format_args!(
            "tnpm: could not update registry cache: {error}"
        ));
    }
}
fn sort_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut parsed: Vec<_> = names.map(|name| (name, Version::parse(name))).collect();
    // Match the persistent index, including SemVer ties (e.g. build metadata).
    parsed.sort_by(|(a_name, a), (b_name, b)| match (a, b) {
        (Some(a), Some(b)) => a.compare(b).then_with(|| a_name.cmp(b_name)),
        _ => a_name.cmp(b_name),
    });
    parsed.into_iter().map(|(name, _)| name).collect()
}

fn exact_name<'a>(metadata: &'a Value, selector: &str) -> Option<Cow<'a, str>> {
    metadata
        .get("dist-tags")
        .and_then(|t| t.str(selector))
        .map(Cow::Borrowed)
        .or_else(|| {
            Version::parse(selector)
                .map(|_| Cow::Owned(selector.trim_start_matches('v').to_owned()))
        })
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
    let members = versions.members();
    // Match `Value::get` (first entry wins) while avoiding a linear scan per
    // selected version, which is quadratic for broad ranges.
    let mut index: FxHashMap<&str, &Value> = FxHashMap::default();
    for (name, entry) in members {
        index.entry(name.as_str()).or_insert(entry);
    }
    let mut selected: Vec<&str> = Vec::new();
    let mut sorted_names: Option<Vec<&str>> = None;
    if let Some(exact) = exact_name(value, query.selector) {
        selected.push(
            members
                .iter()
                .find(|(name, _)| name.as_str() == exact.as_ref())
                .map(|(name, _)| name.as_str())
                .with_context(|| format!("version not found: {}", query.selector))?,
        );
    } else {
        let range = Range::parse(query.selector).with_context(|| {
            format!(
                "unknown tag or unsupported version range: {}",
                query.selector
            )
        })?;
        if query.latest_only {
            // `latest` only needs the greatest match; skip sorting every version.
            let mut best: Option<(&str, Version)> = None;
            for (name, _) in members {
                let Some(version) = Version::parse(name) else {
                    bail!(
                        "unknown tag or unsupported version range: {}",
                        query.selector
                    );
                };
                if range.matches(&version)
                    && best
                        .as_ref()
                        .is_none_or(|(_, current)| version.compare(current) == Ordering::Greater)
                {
                    best = Some((name, version));
                }
            }
            selected.push(
                best.map(|(name, _)| name)
                    .with_context(|| format!("no matching version: {}", query.selector))?,
            );
        } else {
            let mut matched: FxHashMap<&str, ()> = FxHashMap::default();
            for (name, _) in members {
                match range.matches_str(name) {
                    Some(true) => {
                        matched.insert(name.as_str(), ());
                    }
                    Some(false) => {}
                    None => bail!(
                        "unknown tag or unsupported version range: {}",
                        query.selector
                    ),
                }
            }
            if matched.is_empty() {
                bail!("no matching version: {}", query.selector);
            }
            let sorted = sort_names(members.iter().map(|(key, _)| key.as_str()));
            selected = sorted
                .iter()
                .copied()
                .filter(|name| matched.contains_key(name))
                .collect();
            sorted_names = Some(sorted);
        }
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
        let names =
            sorted_names.unwrap_or_else(|| sort_names(members.iter().map(|(key, _)| key.as_str())));
        metadata.set(
            "versions",
            Value::Array(names.into_iter().map(Value::from).collect()),
        );
    }
    let selected = selected
        .into_iter()
        .map(|name| {
            index
                .get(name)
                .map(|value| (*value).clone())
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
        let range = Range::parse(query.selector).with_context(|| {
            format!(
                "unknown tag or unsupported version range: {}",
                query.selector
            )
        })?;
        let count = document.version_count();
        let early_exit = query.latest_only && document.all_versions_valid();
        for n in 0..count {
            let index = if early_exit { count - 1 - n } else { n };
            let (name, node) = document.version(index)?;
            match range.matches_str(name) {
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
