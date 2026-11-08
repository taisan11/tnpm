//! Transparent tsc -> tsrs substitution for package scripts.
//! A temporary PATH shim leaves package.json and shell expressions untouched.

use crate::{config::Config, json::Value, scripts};
use anyhow::{Context, Result};
use std::{env, ffi::OsString, fs, path::Path, process::Command};

#[cfg(tnpm_bundled_tsrs)]
const TSRS_VERSION: &str = "0.23.0-ts7.1.0-beta";

#[cfg(tnpm_bundled_tsrs)]
const BUNDLE: &[u8] = include_bytes!(env!("TNPM_TSRS_BUNDLE"));

/// Called only for explicitly invoked package scripts, not install hooks.
pub fn shim(root: &Path, config: &Config) -> Result<Option<tempfile::TempDir>> {
    let mode = config.get("toolchain-tsrs").unwrap_or("auto");
    if mode == "off"
        || mode == "false"
        || (mode == "auto" && !typescript_71(root))
        || !available(root)
    {
        return Ok(None);
    }

    let dir = tempfile::Builder::new().prefix("tnpm-tsrs-").tempdir()?;
    let binary = env::current_exe().context("could not locate tnpm executable")?;
    #[cfg(windows)]
    {
        let binary = binary.to_string_lossy();
        if binary.contains('"') {
            anyhow::bail!("tnpm executable path contains a quote");
        }
        fs::write(
            dir.path().join("tsc.cmd"),
            format!(
                "@echo off\r\n\"{binary}\" --tnpm-tsrs-shim \"{}\" %*\r\nexit /b %ERRORLEVEL%\r\n",
                dir.path().display()
            ),
        )?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let quoted = binary.to_string_lossy().replace('\'', "'\\''");
        let path = dir.path().join("tsc");
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nexec '{quoted}' --tnpm-tsrs-shim '{}' \"$@\"\n",
                dir.path().display().to_string().replace('\'', "'\\''")
            ),
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(Some(dir))
}

fn typescript_71(root: &Path) -> bool {
    root.ancestors().any(|ancestor| {
        let path = ancestor.join("node_modules/typescript/package.json");
        let Ok(text) = fs::read_to_string(path) else {
            return false;
        };
        let Ok(manifest) = Value::parse(&text) else {
            return false;
        };
        let Some(version) = manifest.str("version") else {
            return false;
        };
        let mut components = version.trim_start_matches('v').split('.');
        matches!(
            (components.next(), components.next()),
            (Some("7"), Some("1"))
        )
    })
}

fn available(root: &Path) -> bool {
    if cfg!(tnpm_bundled_tsrs) {
        return true;
    }
    let filename = if cfg!(windows) { "tsrs.cmd" } else { "tsrs" };
    root.ancestors()
        .map(|ancestor| ancestor.join("node_modules/.bin").join(filename))
        .any(|path| path.is_file())
        || env::split_paths(&env::var_os("PATH").unwrap_or_default()).any(|dir| {
            dir.join(filename).is_file() || (cfg!(windows) && dir.join("tsrs.exe").is_file())
        })
}

/// Internal invocation reached from the temporary tsc shim. Exit code is
/// propagated through scripts::CommandFailed by main.rs.
pub fn dispatch(shim_dir: &Path, args: Vec<OsString>) -> Result<()> {
    let fallback = args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        matches!(arg.as_ref(), "--init" | "--showConfig")
    });
    let mut command = if fallback {
        let mut command = Command::new("tsc");
        // Remove only our shim; otherwise a fallback would recurse.
        let paths = env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .filter(|path| path != shim_dir)
            .collect::<Vec<_>>();
        command.env("PATH", env::join_paths(paths)?);
        command
    } else if let Some(binary) = bundled_binary()? {
        Command::new(binary)
    } else {
        Command::new("tsrs")
    };
    let status = command
        .args(args)
        .status()
        .context("could not launch TypeScript checker")?;
    scripts::check(status)
}

#[cfg(tnpm_bundled_tsrs)]
fn bundled_binary() -> Result<Option<std::path::PathBuf>> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let home = dirs::cache_dir().unwrap_or_else(env::temp_dir);
    let root = home.join("tnpm/toolchain/tsrs");
    let location = root.join(format!("{TSRS_VERSION}-{}", env!("TNPM_TSRS_TARGET")));
    let name = if cfg!(windows) { "tsrs.exe" } else { "tsrs" };
    let executable = location.join(name);
    if executable.is_file() {
        return Ok(Some(executable));
    }

    fs::create_dir_all(&root)?;
    let temp = tempfile::Builder::new()
        .prefix("tsrs-unpack-")
        .tempdir_in(&root)?;
    let staging = temp.path().join("payload");
    fs::create_dir(&staging)?;
    Archive::new(GzDecoder::new(BUNDLE))
        .unpack(&staging)
        .context("could not unpack embedded tsrs")?;
    if !staging.join(name).is_file() || !staging.join("lib.d.ts").is_file() {
        anyhow::bail!("embedded tsrs package is missing the executable or TypeScript libraries");
    }

    if let Err(error) = fs::rename(&staging, &location) {
        if !executable.is_file() {
            return Err(error).context("could not install embedded tsrs in cache");
        }
        // Another tnpm invocation finished extracting the same version.
    }
    Ok(Some(executable))
}

#[cfg(not(tnpm_bundled_tsrs))]
fn bundled_binary() -> Result<Option<std::path::PathBuf>> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_installed_typescript_does_not_trigger_auto_mode() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!typescript_71(dir.path()));
    }

    #[test]
    fn auto_mode_requires_71() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("node_modules/typescript/package.json");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        for (version, expected) in [
            ("5.9.3", false),
            ("7.0.0", false),
            ("7.1.0-beta", true),
            ("7.1.0", true),
            ("8.0.0", false),
        ] {
            fs::write(&file, format!(r#"{{"version":"{version}"}}"#)).unwrap();
            assert_eq!(typescript_71(dir.path()), expected, "{version}");
        }
    }
}
