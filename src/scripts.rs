//! Shared process execution, lifecycle hooks, and local binary discovery.
use crate::json::Value;
use anyhow::{Context, Result};
use std::{
    env,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::Mutex,
};

#[derive(Debug)]
pub struct CommandFailed(pub ExitStatus);
impl std::fmt::Display for CommandFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command exited with {}", self.0)
    }
}
impl std::error::Error for CommandFailed {}

pub fn check(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(CommandFailed(status).into())
    }
}

pub fn command(
    root: &Path,
    manifest: &Value,
    event: &str,
    text: &str,
    args: &[String],
    extra: Option<&Path>,
) -> Result<()> {
    command_output(root, manifest, event, text, args, extra, false)
}

fn command_output(
    root: &Path,
    manifest: &Value,
    event: &str,
    text: &str,
    args: &[String],
    extra: Option<&Path>,
    stdout_to_stderr: bool,
) -> Result<()> {
    let mut paths = Vec::new();
    if let Some(extra) = extra {
        paths.push(extra.to_owned());
    }
    paths.extend(root.ancestors().map(|p| p.join("node_modules/.bin")));
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let script = text;
    let text = if args.is_empty() {
        text.to_owned()
    } else {
        format!(
            "{text} {}",
            args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
        )
    };
    #[cfg(not(windows))]
    let mut child = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(text);
        c
    };
    #[cfg(windows)]
    let mut child = {
        let mut c = Command::new("cmd");
        c.arg("/d").arg("/s").arg("/c").arg(text);
        c
    };
    child
        .current_dir(root)
        .env("PATH", env::join_paths(paths)?)
        .env("npm_lifecycle_event", event)
        .env("npm_lifecycle_script", script)
        .env("npm_package_json", root.join("package.json"))
        .env("npm_package_name", manifest.str("name").unwrap_or(""))
        .env("npm_package_version", manifest.str("version").unwrap_or(""))
        .env("npm_execpath", env::current_exe()?)
        .env("INIT_CWD", env::current_dir()?);
    if stdout_to_stderr {
        child.stdout(std::process::Stdio::from(std::io::stderr()));
    }
    let status = if crate::util::style::progress::active() {
        streaming_status(&mut child, stdout_to_stderr)
    } else {
        child.status().map_err(Into::into)
    }
    .with_context(|| format!("could not run {event} script"))?;
    check(status).with_context(|| format!("{event} script failed"))
}

fn forward_output(
    mut input: impl Read,
    mut output: impl Write,
    line_open: &Mutex<bool>,
    terminal: bool,
) -> io::Result<()> {
    let mut buffer = [0; 8192];
    loop {
        let length = match input.read(&mut buffer) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if length == 0 {
            return Ok(());
        }
        let mut open = line_open.lock().unwrap_or_else(|e| e.into_inner());
        if terminal {
            crate::util::style::progress::finish();
        }
        output.write_all(&buffer[..length])?;
        output.flush()?;
        if terminal {
            *open = buffer[length - 1] != b'\n';
        }
    }
}

fn streaming_status(command: &mut Command, stdout_to_stderr: bool) -> Result<ExitStatus> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .context("script stdout pipe is missing")?;
    let stderr = child
        .stderr
        .take()
        .context("script stderr pipe is missing")?;
    let line_open = Mutex::new(false);
    let result = std::thread::scope(|scope| -> Result<ExitStatus> {
        let stdout = scope.spawn(|| {
            if stdout_to_stderr {
                forward_output(stdout, io::stderr(), &line_open, true)
            } else {
                forward_output(stdout, io::stdout(), &line_open, io::stdout().is_terminal())
            }
        });
        let stderr = scope.spawn(|| forward_output(stderr, io::stderr(), &line_open, true));
        let status = child.wait()?;
        stdout
            .join()
            .map_err(|_| anyhow::anyhow!("script stdout forwarding failed"))??;
        stderr
            .join()
            .map_err(|_| anyhow::anyhow!("script stderr forwarding failed"))??;
        Ok(status)
    });
    if *line_open.lock().unwrap_or_else(|e| e.into_inner()) {
        crate::util::style::progress::message(format_args!(""));
    }
    result
}

#[cfg(not(windows))]
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
#[cfg(windows)]
pub fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn lifecycle(root: &Path, manifest: &Value, event: &str) -> Result<()> {
    lifecycle_output(root, manifest, event, false)
}

pub fn lifecycle_output(
    root: &Path,
    manifest: &Value,
    event: &str,
    stdout_to_stderr: bool,
) -> Result<()> {
    if let Some(text) = manifest.get("scripts").and_then(|s| s.str(event)) {
        command_output(root, manifest, event, text, &[], None, stdout_to_stderr)?;
    }
    Ok(())
}

pub fn bins(manifest: &Value) -> Vec<(String, String)> {
    match manifest.get("bin") {
        Some(Value::String(path)) => vec![(
            manifest
                .str("name")
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("")
                .into(),
            path.clone(),
        )],
        Some(Value::Object(members)) => members
            .iter()
            .filter_map(|(name, value)| value.string().map(|s| (name.clone(), s.into())))
            .collect(),
        _ => Vec::new(),
    }
}

pub fn local_bin(root: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
        return None;
    }
    root.ancestors()
        .map(|p| p.join("node_modules/.bin").join(name))
        .find(|p| p.is_file())
}
