//! Manage only the shell configuration block owned by tnpm.
use crate::cli::Options;
use anyhow::Result;
#[cfg(not(windows))]
use anyhow::{Context, bail};
#[cfg(not(windows))]
use std::{env, fs, io::ErrorKind, path::PathBuf};

#[cfg(not(windows))]
const START: &str = "# >>> tnpm npm alias >>>\n";
#[cfg(not(windows))]
const START_JOINED: &str = "# >>> tnpm npm alias (added newline) >>>\n";
#[cfg(not(windows))]
const END: &str = "# <<< tnpm npm alias <<<\n";

#[cfg(windows)]
pub(crate) fn run(options: &Options) -> Result<()> {
    use anyhow::{Context, bail};
    let exe = std::env::current_exe().context("could not locate tnpm executable")?;
    if !exe
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("tnpm.exe"))
    {
        bail!("Windows alias requires the executable to be named tnpm.exe");
    }
    let dir = exe
        .parent()
        .context("tnpm executable has no parent directory")?;
    let powershell = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .context("SystemRoot is not set")?
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let status = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            include_str!("alias_windows.ps1"),
        ])
        .env("TNPM_ALIAS_DIRECTORY", dir)
        .env("TNPM_ALIAS_ACTION", &options.arguments[0])
        .status()
        .context("could not start Windows PowerShell")?;
    if !status.success() {
        bail!("could not update Windows npm alias");
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn run(options: &Options) -> Result<()> {
    let shell =
        env::var_os("SHELL").context("SHELL is not set; supported shells: bash, zsh, fish")?;
    let shell = PathBuf::from(shell);
    let name = shell.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let home = env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .context("HOME is not set")?;
    let home = PathBuf::from(home);
    let path = match name {
        "bash" => home.join(".bashrc"),
        "zsh" => env::var_os("ZDOTDIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home)
            .join(".zshrc"),
        "fish" => env::var_os("XDG_CONFIG_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("fish/config.fish"),
        _ => bail!("unsupported shell {name:?}; supported shells: bash, zsh, fish"),
    };
    let original = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
    };
    let clean = remove_block(&original)?;
    let result = if options.arguments[0] == "create" {
        let joined = !clean.is_empty() && !clean.ends_with('\n');
        format!(
            "{}{}{}alias npm='tnpm'\n{END}",
            clean,
            if joined { "\n" } else { "" },
            if joined { START_JOINED } else { START }
        )
    } else {
        clean
    };
    if result != original {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        fs::write(&path, result).with_context(|| format!("could not write {}", path.display()))?;
    }
    println!(
        "Updated {}. Start a new shell to apply the change.",
        path.display()
    );
    Ok(())
}

#[cfg(not(windows))]
fn remove_block(text: &str) -> Result<String> {
    let starts: Vec<_> = text
        .match_indices(START)
        .map(|(i, _)| (i, false))
        .chain(text.match_indices(START_JOINED).map(|(i, _)| (i, true)))
        .collect();
    let ends: Vec<_> = text.match_indices(END).map(|(i, _)| i).collect();
    if starts.is_empty() && ends.is_empty() {
        return Ok(text.to_owned());
    }
    if starts.len() != 1 || ends.len() != 1 {
        bail!("invalid tnpm alias markers; please inspect your shell configuration");
    }
    let (start, joined) = starts[0];
    let marker_len = if joined {
        START_JOINED.len()
    } else {
        START.len()
    };
    let end = ends[0];
    if (start > 0 && text.as_bytes()[start - 1] != b'\n')
        || end < start + marker_len
        || (end > 0 && text.as_bytes()[end - 1] != b'\n')
        || (joined && start == 0)
    {
        bail!("invalid tnpm alias markers; please inspect your shell configuration");
    }
    if &text[start + marker_len..end] != "alias npm='tnpm'\n" {
        bail!("tnpm alias block was modified; please inspect your shell configuration");
    }
    let start = if joined { start - 1 } else { start };
    Ok(format!("{}{}", &text[..start], &text[end + END.len()..]))
}
