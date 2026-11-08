//! Registry selection and origin/path-scoped credentials shared by all HTTP commands.
use super::Config;
use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Url, header::HeaderValue};
use std::{fs, time::Duration};

const AUTH: &[&str] = &["_authToken", "_auth", "username", "_password"];
const PRIVATE: &[&str] = &[
    "_authToken",
    "_auth",
    "username",
    "_password",
    "certfile",
    "keyfile",
];

fn matches(scope: &str, url: &Url) -> bool {
    scope.starts_with("//")
        && scope.ends_with('/')
        && url
            .as_str()
            .split_once("://")
            .is_some_and(|(_, rest)| rest.starts_with(&scope[2..]))
}

impl Config {
    pub fn registry_url(&self, name: &str, override_url: Option<&str>) -> Result<Url> {
        let scope = name
            .strip_prefix('@')
            .and_then(|n| n.split_once('/'))
            .map(|(scope, _)| format!("@{scope}:registry"));
        let registry = override_url
            .or_else(|| scope.as_deref().and_then(|key| self.get(key)))
            .or_else(|| self.get("registry"))
            .unwrap_or("https://registry.npmjs.org/");
        let mut url = Url::parse(registry).context("invalid registry URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("registry must be an HTTP(S) URL without credentials, query or fragment");
        }
        if !url.path().ends_with('/') {
            url.set_path(&format!("{}/", url.path()));
        }
        url.path_segments_mut()
            .map_err(|_| anyhow!("invalid registry URL"))?
            .pop_if_empty()
            .push(name);
        Ok(url)
    }

    fn credential_scope<'a>(&'a self, url: &Url, fields: &[&str]) -> Option<&'a str> {
        self.values
            .keys()
            .filter_map(|key| {
                let (scope, field) = key.rsplit_once(':')?;
                (fields.contains(&field) && matches(scope, url)).then_some(scope)
            })
            .max_by_key(|scope| scope.len())
    }

    fn scoped<'a>(&'a self, url: &Url, field: &str) -> Option<&'a str> {
        let scope = self.credential_scope(url, &[field])?;
        self.get(&format!("{scope}:{field}"))
    }

    pub fn authenticated(&self, url: &Url) -> bool {
        self.credential_scope(url, PRIVATE).is_some()
    }

    pub fn auth_scope<'a>(&'a self, url: &Url) -> Option<&'a str> {
        self.credential_scope(url, AUTH)
    }

    pub fn client_authenticated(&self, url: &Url) -> bool {
        self.scoped(url, "certfile").is_some() && self.scoped(url, "keyfile").is_some()
    }

    pub fn authorization(&self, url: &Url) -> Result<Option<HeaderValue>> {
        let Some(scope) = self.credential_scope(url, AUTH) else {
            return Ok(None);
        };
        let get = |key: &str| -> Result<Option<&str>> {
            let value = self
                .get(&format!("{scope}:{key}"))
                .filter(|v| !v.is_empty());
            if value.is_some_and(|v| v.contains("${")) {
                bail!("registry authentication environment variable is not set");
            }
            Ok(value)
        };
        let value = if let Some(token) = get("_authToken")? {
            format!("Bearer {token}")
        } else if let Some(auth) = get("_auth")? {
            STANDARD
                .decode(auth)
                .map_err(|_| anyhow!("invalid base64 registry _auth"))?;
            format!("Basic {auth}")
        } else if let (Some(user), Some(password)) = (get("username")?, get("_password")?) {
            let password = STANDARD
                .decode(password)
                .map_err(|_| anyhow!("invalid base64 registry _password"))?;
            let mut credentials = format!("{user}:").into_bytes();
            credentials.extend(password);
            format!("Basic {}", STANDARD.encode(credentials))
        } else {
            return Ok(None);
        };
        let mut header = HeaderValue::from_str(&value)
            .map_err(|_| anyhow!("invalid registry authentication header"))?;
        header.set_sensitive(true);
        Ok(Some(header))
    }

    /// Client pooling must not mix different TLS identities or redirect boundaries.
    pub fn http_client_key(&self, url: &Url) -> String {
        let fields = [
            "_authToken",
            "_auth",
            "username",
            "_password",
            "cafile",
            "certfile",
            "keyfile",
        ];
        format!(
            "{}{}",
            url.origin().ascii_serialization(),
            self.credential_scope(url, &fields).unwrap_or("")
        )
    }

    pub fn http_client(&self, url: &Url, tarball: bool) -> Result<reqwest::Client> {
        self.build_http_client(url, tarball, false, false, false)
    }

    pub fn audit_client(&self, url: &Url) -> Result<reqwest::Client> {
        self.build_http_client(url, false, true, false, false)
    }

    pub fn remote_client(&self, url: &Url) -> Result<reqwest::Client> {
        self.build_http_client(url, false, false, true, false)
    }

    /// Login bodies and publication data must never be replayed to a redirect target.
    pub fn registry_write_client(&self, url: &Url) -> Result<reqwest::Client> {
        self.build_http_client(url, false, false, false, true)
    }

    fn build_http_client(
        &self,
        url: &Url,
        tarball: bool,
        audit: bool,
        remote: bool,
        no_redirect: bool,
    ) -> Result<reqwest::Client> {
        let boundary = self.credential_scope(url, PRIVATE).map(str::to_owned);
        let origin = url.origin();
        let https = url.scheme() == "https";
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if no_redirect {
                    attempt.stop()
                } else if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if remote
                    && (!matches!(attempt.url().scheme(), "http" | "https")
                        || !attempt.url().username().is_empty()
                        || attempt.url().password().is_some()
                        || (https && attempt.url().scheme() != "https"))
                {
                    attempt.error("unsafe remote module redirect")
                } else if audit && attempt.url().origin() != origin {
                    // Audit POST bodies contain private dependency names, even without credentials.
                    attempt.stop()
                } else if boundary.as_deref().is_some_and(|scope| {
                    attempt.url().origin() != origin || !matches(scope, attempt.url())
                }) {
                    // Do not forward credentials (including client certificates) outside their scope.
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }));
        if tarball {
            builder = builder.no_gzip().no_brotli().no_deflate().no_zstd();
        } else {
            builder = crate::registry::transport::Transport::current().configure(builder);
        }
        if let Some(path) = self.scoped(url, "cafile").or_else(|| self.get("cafile")) {
            let pem = fs::read(path).with_context(|| format!("could not read cafile {path}"))?;
            let certificates =
                reqwest::Certificate::from_pem_bundle(&pem).context("invalid cafile PEM")?;
            if certificates.is_empty() {
                bail!("cafile contains no certificates");
            }
            builder = builder.tls_certs_merge(certificates);
        }
        match (self.scoped(url, "certfile"), self.scoped(url, "keyfile")) {
            (Some(cert), Some(key)) => {
                let mut pem = fs::read(cert).context("could not read registry certfile")?;
                pem.push(b'\n');
                pem.extend(fs::read(key).context("could not read registry keyfile")?);
                builder = builder.identity(
                    reqwest::Identity::from_pem(&pem).context("invalid registry TLS identity")?,
                );
            }
            (None, None) => {}
            _ => bail!("registry certfile and keyfile must both be configured"),
        }
        Ok(builder.build()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn credentials_use_longest_scope_and_never_cross_paths_or_origins() {
        let config = Config {
            values: BTreeMap::from([
                ("//registry.test/:_authToken".into(), "host-token".into()),
                ("//registry.test/team/:username".into(), "user".into()),
                (
                    "//registry.test/team/:_password".into(),
                    STANDARD.encode("pass"),
                ),
            ]),
            arrays: BTreeMap::new(),
        };
        let auth = |url| {
            config
                .authorization(&Url::parse(url).unwrap())
                .unwrap()
                .map(|v| v.to_str().unwrap().to_owned())
        };
        assert_eq!(
            auth("https://registry.test/pkg"),
            Some("Bearer host-token".into())
        );
        assert_eq!(
            auth("https://registry.test/team/pkg"),
            Some(format!("Basic {}", STANDARD.encode("user:pass")))
        );
        assert_eq!(
            auth("https://registry.test/team-other/pkg"),
            Some("Bearer host-token".into())
        );
        assert_eq!(auth("https://other.test/team/pkg"), None);
        assert_eq!(auth("https://registry.test.evil.test/pkg"), None);
        assert_eq!(auth("https://registry.test:8443/pkg"), None);
    }
}
