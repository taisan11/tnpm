use crate::support::Fixture;
use std::{fs, process::Command};

// Exercise the production script against an isolated HKCU key, never the real PATH.
#[test]
fn windows_alias_preserves_files_and_restores_the_user_path() {
    let f = Fixture::new();
    let key = format!(
        "Software\\tnpm-test-{}",
        f.root.file_name().unwrap().to_string_lossy()
    );
    let script = include_str!("../../src/commands/alias_windows.ps1").replace(
        "CreateSubKey('Environment')",
        &format!("CreateSubKey('{key}')"),
    );
    let run = |action: &str| {
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env("TNPM_ALIAS_DIRECTORY", &f.root)
            .env("TNPM_ALIAS_ACTION", action)
            .output()
            .unwrap()
    };
    let result = run("create");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(f.root.join("npm.cmd").is_file());
    assert!(f.root.join("npm.ps1").is_file());
    assert!(run("create").status.success());
    fs::write(f.root.join("npm.cmd"), "user content").unwrap();
    assert!(!run("delete").status.success());
    fs::write(
        f.root.join("npm.cmd"),
        "@\"%~dp0tnpm.exe\" %*\r\n@exit /b %errorlevel%\r\n",
    )
    .unwrap();
    assert!(run("delete").status.success());
    assert!(!f.root.join("npm.cmd").exists());
    assert!(run("delete").status.success());
    let cleanup = format!(
        "[Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey('{}', $false)",
        key
    );
    assert!(
        Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &cleanup])
            .status()
            .unwrap()
            .success()
    );
}
