//! Runtime compatibility checks shared by roots and installed packages.
use super::context::Context;
use crate::{json::Value, semver};
use anyhow::{Result, bail};
use std::process::Command;

fn accepts(data: Option<&Value>, current: &str) -> bool {
    let values = match data {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(values)) => values.iter().filter_map(Value::string).collect(),
        _ => return true,
    };
    !values.iter().any(|v| *v == format!("!{current}"))
        && (!values.iter().any(|v| !v.starts_with('!'))
            || values.contains(&current)
            || values.contains(&"any"))
}

pub(super) fn compatible(data: &Value) -> bool {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let cpu = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else {
        "glibc"
    };
    accepts(data.get("os"), os)
        && accepts(data.get("cpu"), cpu)
        && (os != "linux" || accepts(data.get("libc"), libc))
}

pub(super) fn engines(ctx: &Context<'_>, data: &Value, label: &str) -> Result<()> {
    let Some(engines) = data.get("engines") else {
        return Ok(());
    };
    let strict = ctx.options.engine_strict || ctx.config.enabled("engine-strict");
    let force = ctx.options.force || ctx.config.enabled("force");
    for (engine, required) in engines.members() {
        let Some(required) = required.string() else {
            continue;
        };
        let actual = match engine.as_str() {
            "node" => ctx.node_version(),
            "tnpm" => Some(env!("CARGO_PKG_VERSION").to_owned()),
            _ => continue,
        };
        if actual
            .as_deref()
            .is_some_and(|v| semver::satisfies(v, required) == Some(true))
        {
            continue;
        }
        let message = format!(
            "EBADENGINE: {label} requires {engine}@{required}, current {}",
            actual.as_deref().unwrap_or("unavailable")
        );
        if strict && !force {
            bail!("{message}");
        }
        crate::util::style::progress::message(format_args!("tnpm: warning: {message}"));
    }
    Ok(())
}

pub(super) fn node_version() -> Option<String> {
    let out = Command::new("node").arg("--version").output().ok()?;
    out.status.success().then(|| {
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_start_matches('v')
            .to_owned()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn platform_values_accept_strings_arrays_and_negations() {
        assert!(accepts(Some(&Value::from("linux")), "linux"));
        assert!(!accepts(Some(&Value::from("!linux")), "linux"));
        assert!(!accepts(
            Some(&Value::parse(r#"["glibc","!musl"]"#).unwrap()),
            "musl"
        ));
        assert!(accepts(
            Some(&Value::parse(r#"["!musl"]"#).unwrap()),
            "glibc"
        ));
    }
}
