//! URL modules and isolated npm/JSR imports for the synchronous Node loader bridge.
use crate::{cli::Options, config::Config, json::Value, manifest, semver, spec};
use anyhow::{Context, Result, bail, ensure};
use reqwest::Url;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
};

const MAX_SOURCE: usize = 16 * 1024 * 1024;

pub(super) struct Settings {
    pub root: PathBuf,
    pub options: Options,
    pub config: Config,
}

pub(super) fn settings(cwd: &Path, options: &Options, config: &Config) -> Result<Value> {
    let root = crate::config::project_root(cwd);
    let root = crate::workspaces::root(&root)?.unwrap_or(root);
    let mut value = Value::object();
    value.set(
        "root",
        Value::from(root.to_str().context("runner root is not UTF-8")?),
    );
    value.set(
        "offline",
        Value::Bool(options.offline || config.enabled("offline")),
    );
    value.set("noCache", Value::Bool(options.no_cache));
    ensure!(
        !(options.no_cache && (options.offline || config.enabled("offline"))),
        "--offline and --no-cache cannot be combined"
    );
    if let Some(registry) = &options.registry {
        value.set("registry", Value::from(registry.as_str()));
    }
    for field in ["userconfig", "globalconfig"] {
        if let Some(path) = config.get(field) {
            let path = env::current_dir()?.join(path);
            value.set(
                field,
                Value::from(path.to_str().context("config path is not UTF-8")?),
            );
        }
    }
    Ok(value)
}

impl Settings {
    pub(super) fn load() -> Result<Self> {
        let value =
            Value::parse(&env::var("TNPM_RUNNER_CONFIG").context("missing runner configuration")?)?;
        let root = PathBuf::from(value.str("root").context("missing runner root")?);
        let options = Options {
            offline: value.get("offline") == Some(&Value::Bool(true)),
            no_cache: value.get("noCache") == Some(&Value::Bool(true)),
            registry: value.str("registry").map(str::to_owned),
            userconfig: value.str("userconfig").map(str::to_owned),
            globalconfig: value.str("globalconfig").map(str::to_owned),
            ..Default::default()
        };
        let config = Config::load(&root, &options)?;
        ensure!(
            !(options.no_cache && (options.offline || config.enabled("offline"))),
            "--offline and --no-cache cannot be combined"
        );
        Ok(Self {
            root,
            options,
            config,
        })
    }
}

fn checked_url(text: &str) -> Result<Url> {
    let url = Url::parse(text).context("invalid module URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none(),
        "remote modules require HTTP(S) URLs without credentials"
    );
    Ok(url)
}

fn checksum(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn media_extension(url: &Url, media: &str) -> Result<&'static str> {
    let media = media
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    ensure!(
        !matches!(media.as_str(), "text/html" | "application/xhtml+xml"),
        "remote module returned HTML: {url}"
    );
    let path = url.path();
    let extension = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match extension {
        "ts" | "mts" => Ok("ts"),
        "tsx" => Ok("tsx"),
        "jsx" => Ok("jsx"),
        "js" | "mjs" => Ok("js"),
        "json" => Ok("json"),
        "cjs" | "cts" => {
            bail!("remote CommonJS modules are not supported; use an npm: or jsr: import")
        }
        _ => match media.as_str() {
            "application/typescript" | "text/typescript" | "video/mp2t" => Ok("ts"),
            "application/tsx" | "text/tsx" => Ok("tsx"),
            "application/javascript"
            | "text/javascript"
            | "application/ecmascript"
            | "text/ecmascript" => Ok("js"),
            "application/json" | "text/json" => Ok("json"),
            _ => bail!("unsupported remote module media type {media:?}: {url}"),
        },
    }
}

fn cached(path: &Path, requested: &Url) -> Option<Value> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > (MAX_SOURCE * 8) as u64 {
        return None;
    }
    let value = Value::parse(&fs::read_to_string(path).ok()?).ok()?;
    let final_url = checked_url(value.str("url")?).ok()?;
    if value.str("requested") != Some(requested.as_str())
        || (requested.scheme() == "https" && final_url.scheme() != "https")
        || value.str("source")?.len() > MAX_SOURCE
        || value.str("checksum") != Some(checksum(value.str("source")?).as_str())
        || media_extension(&final_url, value.str("media")?).is_err()
    {
        return None;
    }
    Some(value)
}

fn save_cache(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().context("module cache has no parent")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(value.pretty().as_bytes())?;
    temporary.persist(path)?;
    Ok(())
}

pub(super) async fn fetch(settings: &Settings, text: &str) -> Result<Value> {
    let mut requested = checked_url(text)?;
    let fragment = requested.fragment().map(str::to_owned);
    requested.set_fragment(None);
    let authenticated = settings.config.authenticated(&requested);
    let cache = (!settings.options.no_cache && !authenticated)
        .then(|| {
            dirs::home_dir().map(|home| {
                home.join(".tnpm/cache/modules-v1").join(format!(
                    "{:x}.json",
                    Sha256::digest(requested.as_str().as_bytes())
                ))
            })
        })
        .flatten();
    let saved = cache.as_deref().and_then(|path| cached(path, &requested));
    let cache_hit = saved.is_some();
    let mut cacheable = true;
    let value = if let Some(value) = saved {
        value
    } else {
        ensure!(
            !settings.options.offline && !settings.config.enabled("offline"),
            "ENOTCACHED: remote module {requested}"
        );
        let mut request = settings.config.remote_client(&requested)?.get(requested.clone())
            .header(reqwest::header::ACCEPT, "application/typescript, text/typescript, application/javascript, text/javascript, application/json, */*;q=0.1");
        if settings.options.no_cache {
            request = request.header(reqwest::header::CACHE_CONTROL, "no-cache, no-store");
        }
        if let Some(auth) = settings.config.authorization(&requested)? {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let mut response = request
            .send()
            .await
            .context("remote module request failed")?
            .error_for_status()?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "unexpected remote module HTTP status: {}",
            response.status()
        );
        let final_url = checked_url(response.url().as_str())?;
        cacheable = !response
            .headers()
            .get_all(reqwest::header::CACHE_CONTROL)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .flat_map(|h| h.split(','))
            .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"));
        ensure!(
            requested.scheme() != "https" || final_url.scheme() == "https",
            "HTTPS module redirect cannot downgrade to HTTP"
        );
        let media = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_owned();
        media_extension(&final_url, &media)?;
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_SOURCE as u64),
            "remote module exceeds 16 MiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_SOURCE,
                "remote module exceeds 16 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        let source = String::from_utf8(bytes).context("remote module is not valid UTF-8")?;
        let mut value = Value::object();
        value.set("requested", Value::from(requested.as_str()));
        value.set("url", Value::from(final_url.as_str()));
        value.set("media", Value::from(media.as_str()));
        value.set("checksum", Value::from(checksum(&source).as_str()));
        value.set("source", Value::from(source.as_str()));
        value
    };
    let mut final_url = checked_url(value.str("url").context("cached module has no URL")?)?;
    final_url.set_fragment(fragment.as_deref());
    let extension = media_extension(&final_url, value.str("media").unwrap_or(""))?;
    let source = value.str("source").context("cached module has no source")?;
    let code = if extension == "json" {
        Value::parse(source).context("invalid remote JSON module")?;
        source.to_owned()
    } else {
        super::oxc::transform_source(
            Path::new(&format!("remote.{extension}")),
            source.to_owned(),
            false,
        )
        .with_context(|| format!("could not transform remote module {final_url}"))?
    };
    // Never persist invalid sources, HTML, credentials or authenticated responses.
    if !cache_hit
        && cacheable
        && let Some(path) = cache
        && let Err(error) = save_cache(&path, &value)
    {
        eprintln!("tnpm: could not cache remote module: {error}");
    }
    let mut output = Value::object();
    output.set("url", Value::from(final_url.as_str()));
    output.set(
        "format",
        Value::from(if extension == "json" {
            "json"
        } else {
            "module"
        }),
    );
    output.set("source", Value::from(code.as_str()));
    Ok(output)
}

pub(super) async fn package(
    settings: &Settings,
    text: &str,
    parent: &Path,
    conditions: &[String],
) -> Result<PathBuf> {
    let (name, selector, subpath) = if text.starts_with("jsr:") {
        let jsr = crate::jsr::Specifier::parse(text)?;
        (
            jsr.name.to_owned(),
            format!("jsr:{}@{}", jsr.name, jsr.range),
            jsr.subpath.to_owned(),
        )
    } else {
        let text = text
            .strip_prefix("npm:")
            .context("expected npm: or jsr: specifier")?;
        let start = if text.starts_with('@') {
            text.find('/').context("invalid scoped npm import")? + 1
        } else {
            0
        };
        let end = text[start..].find('/').map_or(text.len(), |i| start + i);
        let (name, range) = spec::package_spec(&text[..end])?;
        let range = range.unwrap_or("latest");
        ensure!(
            !range.contains(':'),
            "unsupported npm import version selector"
        );
        let subpath = text.get(end + 1..).unwrap_or("");
        ensure!(
            end == text.len() || !subpath.is_empty(),
            "empty npm export subpath"
        );
        crate::jsr::validate_subpath(subpath)?;
        (name.to_owned(), range.to_owned(), subpath.to_owned())
    };
    let (target, range) = spec::registry_dependency(&name, &selector)?;
    let import = if subpath.is_empty() {
        name.clone()
    } else {
        format!("{name}/{subpath}")
    };
    let parent = if parent.is_absolute() {
        parent.to_owned()
    } else {
        settings.root.join(".tnpm-runner-entry.mjs")
    };
    // Explicit protocol imports may reuse a compatible installation, but never a different alias target.
    let mut at = parent.parent();
    while let Some(directory) = at {
        let package = directory.join("node_modules").join(&name);
        if let Some(data) = manifest::read(&package)?
            && data.str("name") == Some(target.as_ref())
            && (range == "latest"
                || semver::satisfies(data.str("version").unwrap_or(""), range) == Some(true))
        {
            return super::oxc::resolve_package(directory, &import, conditions);
        }
        at = directory.parent();
    }
    let root = crate::install::runtime_package(
        &settings.root,
        &name,
        &selector,
        &settings.options,
        &settings.config,
    )
    .await?;
    super::oxc::resolve_package(&root, &import, conditions)
}
