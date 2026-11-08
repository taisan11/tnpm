//! Registry account authentication; secrets are never printed or saved in projects.
use super::registry;
use crate::{cli::Options, config::Config, json::Value};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Method, StatusCode, Url};
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

pub(crate) async fn run(options: &Options, config: &Config) -> Result<()> {
    ensure!(
        !options.offline && !config.enabled("offline"),
        "authentication commands are unavailable in offline mode"
    );
    let base = registry::base(config, options)?;
    match options.command.as_str() {
        "whoami" => {
            registry::require_auth(config, &base)?;
            let auth = config.authorization(&base)?;
            let basic = auth
                .as_ref()
                .and_then(|h| h.to_str().ok())
                .and_then(|h| h.strip_prefix("Basic "));
            let username = if let Some(basic) = basic {
                let decoded = String::from_utf8(STANDARD.decode(basic)?)
                    .context("invalid Basic credentials")?;
                decoded
                    .split_once(':')
                    .context("invalid Basic credentials")?
                    .0
                    .to_owned()
            } else {
                let url = registry::endpoint(&base, &["-", "whoami"]);
                let data = registry::json(
                    registry::request(config, options, Method::GET, &url, None, true).await?,
                )
                .await?;
                data.str("username")
                    .filter(|s| !s.is_empty())
                    .context("registry response has no username")?
                    .to_owned()
            };
            ensure!(
                !username.is_empty() && !username.chars().any(char::is_control),
                "invalid registry username"
            );
            if options.json {
                println!("{}", Value::from(username.as_str()).pretty());
            } else {
                println!("{username}");
            }
        }
        "logout" => {
            registry::require_auth(config, &base)?;
            let scope = config
                .auth_scope(&base)
                .context("certificate-only logout is not supported; remove the TLS identity configuration manually")?
                .to_owned();
            // Revocation must be authenticated at the same scope as the credential itself.
            if let Some(token) = config
                .get(&format!("{scope}:_authToken"))
                .filter(|t| !t.is_empty())
            {
                let mut credential_base = base.clone();
                let credential_url = Url::parse(&format!("{}:{scope}", base.scheme()))?;
                credential_base.set_path(credential_url.path());
                let url = registry::endpoint(&credential_base, &["-", "user", "token", token]);
                let response =
                    registry::request(config, options, Method::DELETE, &url, None, true).await?;
                registry::check(&response)?;
            }
            config.save_credentials(&scope, None, None)?;
            output(options, "Logged out.");
        }
        "login" | "adduser" => {
            let auth_type = options
                .auth_type
                .as_deref()
                .or_else(|| config.get("auth-type"))
                .unwrap_or("web");
            ensure!(
                matches!(auth_type, "web" | "legacy"),
                "auth-type must be web or legacy"
            );
            let token = if auth_type == "web" {
                match web_login(config, options, &base).await? {
                    Some(token) => token,
                    None => legacy_login(config, options, &base).await?,
                }
            } else {
                legacy_login(config, options, &base).await?
            };
            let key = options
                .scope
                .as_ref()
                .map(|s| format!("@{}:registry", s.trim_start_matches('@')));
            config.save_credentials(
                &registry::scope(&base),
                Some(&token),
                key.as_deref().map(|k| (k, base.as_str())),
            )?;
            output(options, "Logged in.");
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn output(options: &Options, message: &str) {
    if options.json {
        println!("{{\"ok\":true}}");
    } else {
        println!("{message}");
    }
}

fn prompt(label: &str, secret: bool) -> Result<String> {
    let value = if secret && io::stdin().is_terminal() {
        rpassword::prompt_password(format!("{label}: "))?
    } else {
        eprint!("{label}: ");
        io::stderr().flush()?;
        let mut value = String::new();
        ensure!(
            io::stdin().read_line(&mut value)? > 0,
            "missing {label}; use --auth-type=web or supply legacy credentials on stdin"
        );
        value.trim_end_matches(['\r', '\n']).to_owned()
    };
    ensure!(
        !value.is_empty() && !value.chars().any(char::is_control),
        "invalid {label}"
    );
    Ok(value)
}

async fn legacy_login(config: &Config, options: &Options, base: &Url) -> Result<String> {
    let username = prompt("Username", false)?;
    let password = prompt("Password", true)?;
    let user_id = format!("org.couchdb.user:{username}");
    let mut body = Value::object();
    body.set("_id", Value::from(user_id.as_str()));
    body.set("name", Value::from(username.as_str()));
    body.set("password", Value::from(password.as_str()));
    body.set("type", Value::from("user"));
    body.set("roles", Value::Array(Vec::new()));
    if options.command == "adduser" {
        body.set("email", Value::from(prompt("Email", false)?.as_str()));
    }
    let url = registry::endpoint(base, &["-", "user", &user_id]);
    let response =
        registry::request(config, options, Method::PUT, &url, Some(&body), false).await?;
    let response = if response.status() == StatusCode::CONFLICT && options.command == "login" {
        let mut read_url = url.clone();
        read_url.set_query(Some("write=true"));
        let data = registry::json(
            registry::request(config, options, Method::GET, &read_url, None, false).await?,
        )
        .await?;
        let rev = data
            .str("_rev")
            .context("registry login response has no revision")?;
        for (key, value) in data.members() {
            if body.get(key).is_none() || key == "roles" {
                body.set(key, value.clone());
            }
        }
        let mut auth = reqwest::header::HeaderValue::from_str(&format!(
            "Basic {}",
            STANDARD.encode(format!("{username}:{password}"))
        ))?;
        auth.set_sensitive(true);
        let url = registry::endpoint(&url, &["-rev", rev]);
        registry::request_with_auth(config, options, Method::PUT, &url, Some(&body), Some(auth))
            .await?
    } else {
        response
    };
    let data = registry::json(response).await?;
    token(&data)
}

fn token(data: &Value) -> Result<String> {
    data.str("token")
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .context("registry login response has no token")
}

fn web_url(data: &Value, key: &str, base: &Url) -> Result<Url> {
    let url = data
        .str(key)
        .and_then(|s| Url::parse(s).ok())
        .context("invalid web login URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && !(base.scheme() == "https" && url.scheme() != "https"),
        "unsafe web login URL"
    );
    Ok(url)
}

async fn web_login(config: &Config, options: &Options, base: &Url) -> Result<Option<String>> {
    let url = registry::endpoint(base, &["-", "v1", "login"]);
    let response = registry::request(
        config,
        options,
        Method::POST,
        &url,
        Some(&Value::object()),
        false,
    )
    .await?;
    if matches!(response.status().as_u16(), 404 | 405 | 500 | 501) {
        eprintln!("Web login is not supported; using legacy authentication.");
        return Ok(None);
    }
    let data = registry::json(response).await?;
    let login = web_url(&data, "loginUrl", base)?;
    let done = web_url(&data, "doneUrl", base)?;
    ensure!(
        done.origin() == base.origin() && done.path().starts_with(base.path()),
        "web login polling URL must stay within the registry origin and path"
    );
    eprintln!("Open this URL in your browser to log in:\n{login}");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let response = registry::request(config, options, Method::GET, &done, None, false).await?;
        if response.status() == StatusCode::OK {
            return Ok(Some(token(&registry::json(response).await?)?));
        }
        if response.status() != StatusCode::ACCEPTED {
            registry::check(&response)?;
            bail!("invalid web login polling response");
        }
        ensure!(Instant::now() < deadline, "web login timed out");
        let delay = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1)
            .clamp(1, 10);
        tokio::time::sleep(Duration::from_secs(delay)).await;
    }
}
