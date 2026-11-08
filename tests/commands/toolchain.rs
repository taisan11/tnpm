//! PATH-based tsc substitution; all tools are fake, so tests are offline.
use crate::support::Fixture;
use std::{fs, os::unix::fs::PermissionsExt};

fn install_fake_compilers(f: &Fixture) {
    f.file(
        "node_modules/.bin/tsrs",
        "#!/bin/sh\nprintf 'tsrs:%s\\n' \"$*\"\nexit 23\n",
    );
    f.file(
        "node_modules/.bin/tsc",
        "#!/bin/sh\nprintf 'tsc:%s\\n' \"$*\"\n",
    );
    for name in ["tsrs", "tsc"] {
        let path = f.root.join("node_modules/.bin").join(name);
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    f.file(
        "package.json",
        r#"{"scripts":{"check":"tsc --noEmit","config":"tsc --showConfig"}}"#,
    );
}

#[test]
fn explicit_tsrs_mode_replaces_tsc_and_preserves_failure_code() {
    let f = Fixture::new();
    install_fake_compilers(&f);
    f.file(".npmrc", "toolchain-tsrs=on\n");
    let out = f.invoke(&["run", "check"]);
    assert_eq!(out.status.code(), Some(23));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "tsrs:--noEmit\n");
}

#[test]
fn unsupported_arguments_fall_back_to_original_tsc() {
    let f = Fixture::new();
    install_fake_compilers(&f);
    f.file(".npmrc", "toolchain-tsrs=on\n");
    assert_eq!(f.ok(&["run", "config"]), "tsc:--showConfig\n");
}

#[test]
fn auto_mode_only_replaces_typescript_71() {
    let f = Fixture::new();
    install_fake_compilers(&f);
    f.file(".npmrc", "toolchain-tsrs=auto\n");
    // No installed TypeScript means no substitution.
    assert_eq!(f.ok(&["run", "check"]), "tsc:--noEmit\n");
    f.file(
        "node_modules/typescript/package.json",
        r#"{"version":"7.1.0-beta"}"#,
    );
    let out = f.invoke(&["run", "check"]);
    assert_eq!(out.status.code(), Some(23));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "tsrs:--noEmit\n");
}

#[test]
fn off_mode_does_not_replace_tsc() {
    let f = Fixture::new();
    install_fake_compilers(&f);
    f.file(
        "node_modules/typescript/package.json",
        r#"{"version":"7.1.0-beta"}"#,
    );
    f.file(".npmrc", "toolchain-tsrs=off\n");
    assert_eq!(f.ok(&["run", "check"]), "tsc:--noEmit\n");
}
