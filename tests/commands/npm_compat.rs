//! CLI integration tests: npm_compat.
use crate::support::Fixture;
use std::fs;

#[test]
#[ignore = "requires Node.js and npm; still only uses a local HTTP registry"]
fn npm_and_tnpm_can_share_v3_lockfiles() {
    let f = Fixture::new();
    let common = f.package("common", "1.0.0", "");
    let app = f.package(
        "app",
        "1.0.0",
        r#", "dependencies":{"common":"^1"},"bin":{"app":"bin/cli.sh"}"#,
    );
    f.registry("common", "1.0.0", &[("1.0.0", common)]);
    f.registry("app", "1.0.0", &[("1.0.0", app)]);
    f.file(
        "package.json",
        r#"{"name":"fixture","version":"1.0.0","dependencies":{"app":"^1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let lock = fs::read_to_string(f.root.join("package-lock.json")).unwrap();
    f.npm(&["ci", "--ignore-scripts", "--no-audit", "--no-fund"]);
    assert_eq!(
        fs::read_to_string(f.root.join("package-lock.json")).unwrap(),
        lock
    );
    fs::remove_file(f.root.join("package-lock.json")).unwrap();
    f.npm(&[
        "install",
        "--package-lock-only",
        "--lockfile-version=3",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
    ]);
    let npm_lock = fs::read_to_string(f.root.join("package-lock.json")).unwrap();
    f.ok(&["ci", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("package-lock.json")).unwrap(),
        npm_lock
    );
    f.ok(&["ls", "--all"]);
}

#[test]
#[ignore = "requires Node.js and npm; still only uses a local HTTP registry"]
fn npm_and_tnpm_share_scoped_cyclic_peer_sets_and_optional_peer_metadata() {
    let f = Fixture::new();
    f.single("@scope/plugin", r#", "peerDependencies":{"@scope/host":"^1","beta":"1","absent":"1"},"peerDependenciesMeta":{"absent":{"optional":true}}"#);
    f.single("@scope/host", "");
    f.single(
        "beta",
        r#", "peerDependencies":{"@scope/host":"1","@scope/plugin":"1"}"#,
    );
    f.file("package.json", r#"{"dependencies":{"@scope/plugin":"1"}}"#);
    f.ok(&["install", "--ignore-scripts", "--strict-peer-deps"]);
    f.ok(&["ls", "--all"]);
    f.npm(&["ci", "--ignore-scripts", "--no-audit", "--no-fund"]);
    fs::remove_file(f.root.join("package-lock.json")).unwrap();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    f.npm(&["install", "--ignore-scripts", "--no-audit", "--no-fund"]);
    let lock = fs::read_to_string(f.root.join("package-lock.json")).unwrap();
    f.ok(&["ci", "--ignore-scripts", "--strict-peer-deps"]);
    assert_eq!(
        fs::read_to_string(f.root.join("package-lock.json")).unwrap(),
        lock
    );
    f.ok(&["ls", "--all"]);
}
