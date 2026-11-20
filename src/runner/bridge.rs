//! Long-lived bridge for the Node synchronous loader hooks.
//!
//! Spawning `tnpm` once per module dominates runner startup, so the loader
//! starts one process and exchanges length-prefixed JSON frames over its stdin
//! and stdout. Results are memoized because Node may resolve or load the same
//! module more than once (dynamic imports, query strings, require + import).
use super::{oxc, remote};
use crate::json::Value;
use anyhow::{Context, Result, bail};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

const MAX_FRAME: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 4096;

type TransformKey = (String, Option<SystemTime>, u64, bool);
type ResolveKey = (String, String, Vec<String>);

pub fn run() -> Result<()> {
    let settings = remote::Settings::load()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut transforms: HashMap<TransformKey, String> = HashMap::new();
    let mut resolutions: HashMap<ResolveKey, PathBuf> = HashMap::new();
    let mut packages: HashMap<ResolveKey, PathBuf> = HashMap::new();

    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    while let Some(frame) = read_frame(&mut stdin)? {
        let response = match Value::parse(&frame) {
            Ok(request) => dispatch(
                &runtime,
                &settings,
                &request,
                &mut transforms,
                &mut resolutions,
                &mut packages,
            ),
            Err(error) => Err(error),
        };
        write_frame(&mut stdout, &response.unwrap_or_else(error_response))?;
    }
    Ok(())
}

fn dispatch(
    runtime: &tokio::runtime::Runtime,
    settings: &remote::Settings,
    request: &Value,
    transforms: &mut HashMap<TransformKey, String>,
    resolutions: &mut HashMap<ResolveKey, PathBuf>,
    packages: &mut HashMap<ResolveKey, PathBuf>,
) -> Result<Value> {
    match request.str("op").unwrap_or("") {
        "transform" => {
            let path = request.str("path").context("transform requires path")?;
            let commonjs = request.str("format") == Some("commonjs");
            let metadata = std::fs::metadata(path)?;
            let key = (
                path.to_owned(),
                metadata.modified().ok(),
                metadata.len(),
                commonjs,
            );
            if let Some(code) = transforms.get(&key) {
                return Ok(string_response("code", code));
            }
            let code = oxc::transform(Path::new(path), commonjs)?;
            if transforms.len() < MAX_ENTRIES {
                transforms.insert(key, code.clone());
            }
            Ok(string_response("code", &code))
        }
        "resolve" => {
            let key = key_from(request)?;
            if let Some(path) = resolutions.get(&key) {
                return path_response(path);
            }
            let path = oxc::resolve(Path::new(&key.0), &key.1, &key.2)?;
            let response = path_response(&path)?;
            if resolutions.len() < MAX_ENTRIES {
                resolutions.insert(key, path);
            }
            Ok(response)
        }
        "package" => {
            let key = key_from(request)?;
            if let Some(path) = packages.get(&key) {
                return path_response(path);
            }
            let path =
                runtime.block_on(remote::package(settings, &key.1, Path::new(&key.0), &key.2))?;
            let response = path_response(&path)?;
            if packages.len() < MAX_ENTRIES {
                packages.insert(key, path);
            }
            Ok(response)
        }
        "fetch" => {
            let url = request.str("url").context("fetch requires url")?;
            // The Node loader already memoizes remote sources for the run.
            let value = runtime.block_on(remote::fetch(settings, url))?;
            Ok(value_response(&value))
        }
        op => bail!("unknown runner bridge operation: {op}"),
    }
}

fn key_from(request: &Value) -> Result<ResolveKey> {
    let parent = request.str("parent").context("request requires parent")?;
    let specifier = request
        .str("specifier")
        .context("request requires specifier")?;
    let conditions = request
        .get("conditions")
        .map(Value::members)
        .unwrap_or_default()
        .iter()
        .filter_map(|(_, value)| value.string().map(str::to_owned))
        .collect();
    Ok((parent.to_owned(), specifier.to_owned(), conditions))
}

fn path_response(path: &Path) -> Result<Value> {
    Ok(string_response(
        "path",
        path.to_str().context("resolved path is not UTF-8")?,
    ))
}

fn string_response(key: &str, value: &str) -> Value {
    let mut response = Value::object();
    response.set("ok", Value::Bool(true));
    response.set(key, Value::from(value));
    response
}

fn value_response(value: &Value) -> Value {
    let mut response = Value::object();
    response.set("ok", Value::Bool(true));
    response.set("value", value.clone());
    response
}

fn error_response(error: anyhow::Error) -> Value {
    let mut response = Value::object();
    response.set("ok", Value::Bool(false));
    response.set("error", Value::String(format!("{error:#}")));
    response
}

fn read_frame(input: &mut impl Read) -> Result<Option<String>> {
    let mut header = [0u8; 4];
    if !read_exact(input, &mut header)? {
        return Ok(None);
    }
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_FRAME {
        bail!("runner bridge request exceeds {MAX_FRAME} bytes");
    }
    let mut payload = vec![0u8; length];
    if !read_exact(input, &mut payload)? {
        bail!("truncated runner bridge request");
    }
    Ok(Some(
        String::from_utf8(payload).context("runner bridge request is not UTF-8")?,
    ))
}

fn read_exact(input: &mut impl Read, buffer: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buffer.len() {
        match input.read(&mut buffer[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "runner bridge frame ended early",
                ));
            }
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(true)
}

fn write_frame(output: &mut impl Write, response: &Value) -> Result<()> {
    let text = nojson::Json(response).to_string();
    let length = u32::try_from(text.len()).context("runner bridge response is too large")?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(text.as_bytes())?;
    output.flush()?;
    Ok(())
}
