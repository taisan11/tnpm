//! Uncached registry operations with bounded credential scope and redacted errors.
use crate::{cli::Options, config::Config, json::Value};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Method, Response, Url};

pub(super) fn endpoint(base: &Url, segments: &[&str]) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("HTTP registry URL")
        .pop_if_empty()
        .extend(segments.iter().copied());
    url
}

pub(super) fn base(config: &Config, options: &Options) -> Result<Url> {
    let name = options
        .scope
        .as_deref()
        .map(|s| format!("@{}/package", s.trim_start_matches('@')))
        .unwrap_or_default();
    let mut url = config.registry_url(&name, options.registry.as_deref())?;
    if !name.is_empty() {
        url.path_segments_mut().unwrap().pop().push("");
    }
    Ok(url)
}

pub(super) fn scope(url: &Url) -> String {
    url.as_str()
        .split_once("://")
        .map(|(_, rest)| format!("//{rest}"))
        .expect("HTTP registry URL")
}

pub(super) async fn request(
    config: &Config,
    options: &Options,
    method: Method,
    url: &Url,
    body: Option<&Value>,
    authenticated: bool,
) -> Result<Response> {
    let auth = if authenticated {
        config.authorization(url)?
    } else {
        None
    };
    request_with_auth(config, options, method, url, body, auth).await
}

pub(super) async fn request_with_auth(
    config: &Config,
    options: &Options,
    method: Method,
    url: &Url,
    body: Option<&Value>,
    auth: Option<reqwest::header::HeaderValue>,
) -> Result<Response> {
    ensure!(
        !options.offline && !config.enabled("offline"),
        "registry operation is unavailable in offline mode"
    );
    let mut request = config
        .registry_write_client(url)?
        .request(method, url.clone())
        .header("accept", "application/json")
        .header("user-agent", concat!("tnpm/", env!("CARGO_PKG_VERSION")));
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    if let Some(otp) = options.otp.as_deref().or_else(|| config.get("otp")) {
        let mut value =
            reqwest::header::HeaderValue::from_str(otp).context("invalid OTP header")?;
        value.set_sensitive(true);
        request = request.header("npm-otp", value);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.pretty());
    }
    // Never include URLs (logout paths contain tokens), bodies, or server error text.
    request
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("registry request failed"))
}

pub(super) fn check(response: &Response) -> Result<()> {
    let status = response.status();
    if !status.is_success() {
        bail!(
            "registry returned HTTP {}; check credentials, permissions and --otp",
            status.as_u16()
        );
    }
    Ok(())
}

pub(super) async fn json(response: Response) -> Result<Value> {
    check(&response)?;
    let bytes = response
        .bytes()
        .await
        .map_err(|_| anyhow::anyhow!("could not read registry response"))?;
    // Do not expose malformed responses that may contain submitted credentials.
    let text =
        std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("invalid registry response"))?;
    Value::parse(text).map_err(|_| anyhow::anyhow!("invalid registry JSON response"))
}

pub(super) fn require_auth(config: &Config, url: &Url) -> Result<()> {
    ensure!(
        config.authorization(url)?.is_some() || config.client_authenticated(url),
        "authentication required; run tnpm login or configure registry-scoped credentials"
    );
    Ok(())
}
