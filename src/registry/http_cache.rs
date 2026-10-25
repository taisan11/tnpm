//! Private HTTP metadata-cache policy. Stale bodies are only reused after validation.
use crate::json::Value;
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RESPONSE_HEADERS: &[&str] = &[
    "cache-control",
    "date",
    "expires",
    "age",
    "etag",
    "last-modified",
    "vary",
];
const REQUEST_HEADERS: &[&str] = &["accept", "accept-encoding", "user-agent"];
const MIN_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

pub fn request_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        header::USER_AGENT,
        HeaderValue::from_static(concat!("tnpm/", env!("CARGO_PKG_VERSION"))),
    );
    // Explicitly persist the same value that is sent, so Vary works across upgrades.
    headers.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static(super::transport::Transport::Default.accept_encoding()),
    );
    headers
}

#[derive(Clone)]
pub struct Policy {
    headers: HeaderMap,
    request_headers: HeaderMap,
    request_time: SystemTime,
    response_time: SystemTime,
    lifetime: Duration,
    initial_age: Duration,
    revalidate: bool,
    storable: bool,
}

impl Policy {
    pub fn new(
        headers: &HeaderMap,
        request: &HeaderMap,
        request_time: SystemTime,
        response_time: SystemTime,
    ) -> Self {
        let headers = only(headers, RESPONSE_HEADERS);
        let request_headers = only(request, REQUEST_HEADERS);
        let mut no_store = false;
        let mut revalidate = false;
        let mut must_revalidate = false;
        let mut max_age = None;
        let mut seen_max_age = false;
        for value in headers.get_all(header::CACHE_CONTROL) {
            let Some(parts) = value.to_str().ok().and_then(directives) else {
                no_store = true;
                continue;
            };
            for part in parts {
                let (name, value) = part
                    .split_once('=')
                    .map(|(n, v)| (n.trim(), Some(v.trim())))
                    .unwrap_or((part.trim(), None));
                if name.eq_ignore_ascii_case("no-store") {
                    no_store = true;
                }
                if name.eq_ignore_ascii_case("no-cache") {
                    revalidate = true;
                }
                if name.eq_ignore_ascii_case("must-revalidate") {
                    must_revalidate = true;
                }
                if name.eq_ignore_ascii_case("max-age") {
                    if seen_max_age {
                        revalidate = true;
                    }
                    seen_max_age = true;
                    max_age = value.and_then(|v| {
                        let value = if let Some(quoted) = v.strip_prefix('"') {
                            quoted.strip_suffix('"')?
                        } else {
                            v
                        };
                        delta_seconds(value)
                    });
                    if max_age.is_none() {
                        revalidate = true;
                    }
                }
                // s-maxage/proxy-revalidate apply to shared caches, not this private cache.
            }
        }
        let date_value = date(&headers, header::DATE);
        if headers.contains_key(header::DATE) && date_value.is_none() {
            revalidate = true;
        }
        let expires = date(&headers, header::EXPIRES);
        if !seen_max_age && headers.contains_key(header::EXPIRES) && expires.is_none() {
            revalidate = true;
        }
        let lifetime = if seen_max_age {
            Duration::from_secs(max_age.unwrap_or(0))
        } else {
            expires
                .and_then(|expires| {
                    expires
                        .duration_since(date_value.unwrap_or(response_time))
                        .ok()
                })
                .unwrap_or_default()
        };
        let apparent_age = date_value
            .and_then(|date| response_time.duration_since(date).ok())
            .unwrap_or_default();
        let response_delay = response_time
            .duration_since(request_time)
            .unwrap_or_default();
        let age = if headers.contains_key(header::AGE) {
            single(&headers, header::AGE)
                .and_then(delta_seconds)
                .map(Duration::from_secs)
                .unwrap_or(Duration::MAX)
        } else {
            Duration::ZERO
        };
        let initial_age = apparent_age.max(age.saturating_add(response_delay));
        // Floor the remaining TTL at receipt, not the origin's total max-age.
        // Keep explicit storage/revalidation restrictions and malformed headers strict.
        let lifetime = if !no_store && !revalidate && !must_revalidate {
            lifetime.max(initial_age.saturating_add(MIN_CACHE_TTL))
        } else {
            lifetime
        };
        let valid_text = headers
            .values()
            .chain(request_headers.values())
            .all(|v| v.to_str().is_ok());
        let vary_supported = vary(&headers).is_some();
        let validators =
            etag(&headers).is_some() || date(&headers, header::LAST_MODIFIED).is_some();
        Self {
            headers,
            request_headers,
            request_time,
            response_time,
            lifetime,
            initial_age,
            revalidate,
            storable: !no_store
                && valid_text
                && vary_supported
                && (!lifetime.is_zero() || validators),
        }
    }

    pub fn can_store(&self) -> bool {
        self.storable
    }

    pub fn matches(&self, request: &HeaderMap) -> bool {
        vary(&self.headers).is_some_and(|names| {
            names.iter().all(|name| {
                self.request_headers
                    .get_all(name)
                    .iter()
                    .eq(request.get_all(name).iter())
            })
        })
    }

    pub fn is_fresh(&self, now: SystemTime) -> bool {
        self.storable
            && !self.revalidate
            && now
                .duration_since(self.response_time)
                .is_ok_and(|resident| self.initial_age.saturating_add(resident) < self.lifetime)
    }

    pub fn conditional_headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if etag(&self.headers).is_some() {
            headers.insert(header::IF_NONE_MATCH, self.headers[header::ETAG].clone());
        } else if date(&self.headers, header::LAST_MODIFIED).is_some() {
            headers.insert(
                header::IF_MODIFIED_SINCE,
                self.headers[header::LAST_MODIFIED].clone(),
            );
        }
        headers
    }

    pub fn accepts_not_modified(&self, headers: &HeaderMap) -> bool {
        match (etag(&self.headers), headers.get(header::ETAG)) {
            (Some(old), Some(_)) => etag(headers)
                .is_some_and(|new| old.trim_start_matches("W/") == new.trim_start_matches("W/")),
            _ => true,
        }
    }

    pub fn revalidated(
        &self,
        headers: &HeaderMap,
        request: &HeaderMap,
        request_time: SystemTime,
        response_time: SystemTime,
    ) -> Self {
        let mut merged = self.headers.clone();
        for name in RESPONSE_HEADERS {
            if headers.contains_key(*name) {
                merged.remove(*name);
                for value in headers.get_all(*name) {
                    merged.append(HeaderName::from_static(name), value.clone());
                }
            }
        }
        // A validation response starts a new age calculation, not the original CDN Age.
        if !headers.contains_key(header::AGE) {
            merged.remove(header::AGE);
        }
        if !headers.contains_key(header::DATE) {
            merged.insert(
                header::DATE,
                HeaderValue::from_str(&httpdate::fmt_http_date(response_time))
                    .expect("HTTP date is a valid header"),
            );
        }
        Self::new(&merged, request, request_time, response_time)
    }

    pub fn response_seconds(&self) -> u64 {
        self.response_time
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn expires_seconds(&self) -> u64 {
        let remaining = if self.revalidate {
            Duration::ZERO
        } else {
            self.lifetime.saturating_sub(self.initial_age)
        };
        self.response_seconds().saturating_add(remaining.as_secs())
    }

    pub fn to_value(&self) -> Value {
        let mut value = Value::object();
        for (name, headers) in [
            ("response-headers", &self.headers),
            ("request-headers", &self.request_headers),
        ] {
            let mut object = Value::object();
            for key in headers.keys() {
                object.set(
                    key.as_str(),
                    Value::Array(
                        headers
                            .get_all(key)
                            .iter()
                            .map(|v| Value::from(v.to_str().expect("cached headers are ASCII")))
                            .collect(),
                    ),
                );
            }
            value.set(name, object);
        }
        for (name, time) in [
            ("request-time-ms", self.request_time),
            ("response-time-ms", self.response_time),
        ] {
            let ms = time
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            value.set(name, Value::Number(ms.to_string()));
        }
        value
    }

    pub fn from_value(value: &Value) -> Option<Self> {
        fn headers(value: &Value, allowed: &[&str]) -> Option<HeaderMap> {
            let Value::Object(members) = value else {
                return None;
            };
            let mut headers = HeaderMap::new();
            for (name, values) in members {
                if !allowed.contains(&name.as_str()) {
                    return None;
                }
                let Value::Array(values) = values else {
                    return None;
                };
                for value in values {
                    headers.append(
                        HeaderName::from_bytes(name.as_bytes()).ok()?,
                        HeaderValue::from_str(value.string()?).ok()?,
                    );
                }
            }
            Some(headers)
        }
        fn time(value: &Value) -> Option<SystemTime> {
            let Value::Number(text) = value else {
                return None;
            };
            UNIX_EPOCH.checked_add(Duration::from_millis(text.parse().ok()?))
        }
        Some(Self::new(
            &headers(value.get("response-headers")?, RESPONSE_HEADERS)?,
            &headers(value.get("request-headers")?, REQUEST_HEADERS)?,
            time(value.get("request-time-ms")?)?,
            time(value.get("response-time-ms")?)?,
        ))
    }
}

fn only(headers: &HeaderMap, names: &[&'static str]) -> HeaderMap {
    let mut result = HeaderMap::new();
    for name in names {
        for value in headers.get_all(*name) {
            result.append(HeaderName::from_static(name), value.clone());
        }
    }
    result
}

fn single(headers: &HeaderMap, name: HeaderName) -> Option<&str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        None
    } else {
        Some(value)
    }
}

fn date(headers: &HeaderMap, name: HeaderName) -> Option<SystemTime> {
    httpdate::parse_http_date(single(headers, name)?).ok()
}

fn etag(headers: &HeaderMap) -> Option<&str> {
    let tag = single(headers, header::ETAG)?;
    let opaque = tag
        .strip_prefix("W/")
        .unwrap_or(tag)
        .strip_prefix('"')?
        .strip_suffix('"')?;
    opaque
        .bytes()
        .all(|b| b == 0x21 || (0x23..=0x7e).contains(&b))
        .then_some(tag)
}

fn delta_seconds(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn vary(headers: &HeaderMap) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for value in headers.get_all(header::VARY) {
        for name in value
            .to_str()
            .ok()?
            .split(',')
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            let name = name.to_ascii_lowercase();
            if !REQUEST_HEADERS.contains(&name.as_str()) {
                return None;
            }
            names.push(name);
        }
    }
    Some(names)
}

fn directives(text: &str) -> Option<Vec<&str>> {
    let (mut quoted, mut escaped, mut start) = (false, false, 0);
    let mut parts = Vec::new();
    for (index, byte) in text.bytes().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' if quoted => escaped = true,
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                parts.push(&text[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if quoted || escaped {
        return None;
    }
    parts.push(&text[start..]);
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn time(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }
    fn policy(items: &[(&'static str, &str)]) -> Policy {
        let mut headers = HeaderMap::new();
        for (name, value) in items {
            headers.append(
                HeaderName::from_static(name),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        Policy::new(&headers, &request_headers(), time(998), time(1000))
    }

    #[test]
    fn max_age_accounts_for_age_date_and_response_delay() {
        let p = policy(&[
            ("cache-control", "public, max-age=600"),
            ("age", "100"),
            ("date", &httpdate::fmt_http_date(time(950))),
        ]);
        assert!(p.is_fresh(time(1497)));
        assert!(!p.is_fresh(time(1498)));
        let p = policy(&[
            ("cache-control", "max-age=600"),
            ("age", "10"),
            ("date", &httpdate::fmt_http_date(time(800))),
        ]);
        assert!(p.is_fresh(time(1399)));
        assert!(!p.is_fresh(time(1400)));
    }

    #[test]
    fn expires_is_used_without_max_age_and_remaining_ttl_has_a_floor() {
        let expires = httpdate::fmt_http_date(time(1600));
        let p = policy(&[
            ("date", &httpdate::fmt_http_date(time(1000))),
            ("expires", &expires),
        ]);
        assert!(p.is_fresh(time(1597)));
        assert!(!p.is_fresh(time(1598)));
        let p = policy(&[("expires", &expires), ("cache-control", "max-age=0")]);
        assert!(p.is_fresh(time(1299)));
        assert!(!p.is_fresh(time(1300)));
        assert!(policy(&[]).can_store());
    }

    #[test]
    fn no_cache_stores_validators_but_no_store_never_stores() {
        let p = policy(&[
            ("cache-control", "no-cache, max-age=600"),
            ("etag", "\"v1\""),
        ]);
        assert!(p.can_store());
        assert!(!p.is_fresh(time(1000)));
        assert_eq!(p.conditional_headers()[header::IF_NONE_MATCH], "\"v1\"");
        assert!(
            !policy(&[
                ("cache-control", "no-store, max-age=600"),
                ("etag", "\"v1\"")
            ])
            .can_store()
        );
        assert!(
            policy(&[("cache-control", "private, max-age=172800, s-maxage=0")])
                .is_fresh(time(100000))
        );
    }

    #[test]
    fn malformed_or_duplicate_max_age_and_unsupported_vary_are_conservative() {
        for value in [
            "max-age=600, max-age=900",
            "max-age=-1",
            "max-age=oops",
            "max-age=\"600",
            "max-age=+60",
            "max-age=\"\"600\"\"",
        ] {
            assert!(
                !policy(&[("cache-control", value), ("etag", "\"v1\"")]).is_fresh(time(1000)),
                "{value}"
            );
        }
        assert!(!policy(&[("cache-control", "max-age=600"), ("vary", "*")]).can_store());
        let p = policy(&[
            ("cache-control", "max-age=600"),
            ("vary", "Accept, User-Agent"),
        ]);
        assert!(p.matches(&request_headers()));
        let mut request = request_headers();
        request.insert(header::ACCEPT, HeaderValue::from_static("text/plain"));
        assert!(!p.matches(&request));
    }

    #[test]
    fn validators_and_serialization_survive_revalidation() {
        let p = policy(&[
            ("cache-control", "max-age=0"),
            ("etag", "\"v1\""),
            ("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
        ]);
        let p = Policy::from_value(&p.to_value()).unwrap();
        assert!(p.conditional_headers().contains_key(header::IF_NONE_MATCH));
        assert!(
            !p.conditional_headers()
                .contains_key(header::IF_MODIFIED_SINCE)
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=600"),
        );
        let p = p.revalidated(&headers, &request_headers(), time(2000), time(2001));
        assert!(p.is_fresh(time(2002)));
        assert_eq!(p.conditional_headers()[header::IF_NONE_MATCH], "\"v1\"");
        headers.insert(header::ETAG, HeaderValue::from_static("W/\"v1\""));
        assert!(p.accepts_not_modified(&headers));
        headers.insert(header::ETAG, HeaderValue::from_static("\"v2\""));
        assert!(!p.accepts_not_modified(&headers));
    }

    #[test]
    fn short_missing_and_already_aged_ttls_get_five_minutes_from_receipt() {
        let past = httpdate::fmt_http_date(time(500));
        for headers in [
            vec![],
            vec![("cache-control", "max-age=60")],
            vec![("cache-control", "max-age=0")],
            vec![("cache-control", "max-age=600"), ("age", "590")],
            vec![("cache-control", "max-age=600"), ("age", "1000")],
            vec![("expires", past.as_str())],
        ] {
            let p = policy(&headers);
            assert!(p.can_store(), "{headers:?}");
            assert!(p.is_fresh(time(1299)), "{headers:?}");
            assert!(!p.is_fresh(time(1300)), "{headers:?}");
            assert_eq!(p.expires_seconds(), 1300);
            let restored = Policy::from_value(&p.to_value()).unwrap();
            assert!(restored.is_fresh(time(1299)));
            assert!(
                !restored.is_fresh(time(1300)),
                "loading must not restart the five-minute TTL"
            );
        }
    }

    #[test]
    fn minimum_does_not_override_explicit_revalidation_or_storage_restrictions() {
        for control in [
            "no-store, max-age=0",
            "no-cache, max-age=0",
            "must-revalidate, max-age=0",
        ] {
            let p = policy(&[("cache-control", control), ("etag", "\"v1\"")]);
            assert!(!p.is_fresh(time(1000)), "{control}");
        }
        let p = policy(&[("cache-control", "must-revalidate, max-age=60")]);
        assert!(p.is_fresh(time(1057)));
        assert!(!p.is_fresh(time(1058)));
        assert!(!policy(&[("expires", "invalid"), ("etag", "\"v1\"")]).is_fresh(time(1000)));
        assert!(
            !policy(&[("cache-control", "max-age=60"), ("age", "invalid")]).is_fresh(time(1000))
        );
    }

    #[test]
    fn vary_rejects_the_old_gzip_only_request_snapshot() {
        let mut response = HeaderMap::new();
        response.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=600"),
        );
        response.insert(header::VARY, HeaderValue::from_static("Accept-Encoding"));
        let mut old_request = request_headers();
        old_request.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        let policy = Policy::new(&response, &old_request, time(1000), time(1000));
        let restored = Policy::from_value(&policy.to_value()).unwrap();
        assert!(restored.matches(&old_request));
        assert!(!restored.matches(&request_headers()));
    }
}
