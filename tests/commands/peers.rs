//! CLI integration tests: peers.
use crate::support::Fixture;
use std::fs;

#[test]
fn required_peers_are_siblings_optional_peers_are_not_auto_installed() {
    let f = Fixture::new();
    f.single("plugin", r#", "peerDependencies":{"host":"^1","optional-host":"^1"},"peerDependenciesMeta":{"optional-host":{"optional":true}}"#);
    f.single("host", "");
    f.file("package.json", r#"{"devDependencies":{"plugin":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/host/package.json").is_file());
    assert!(
        !f.root
            .join("node_modules/plugin/node_modules/host")
            .exists()
    );
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.contains("optional-host"))
    );
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    assert!(lock.contains("\"peer\":true"));
    f.ok(&["ls", "--all"]);
    f.ok(&["ci", "--ignore-scripts"]);
    f.ok(&["prune", "--omit=dev"]);
    assert!(!f.root.join("node_modules/host").exists());
}

#[test]
fn overlapping_peers_backtrack_to_a_shared_version_and_handle_cycles() {
    let f = Fixture::new();
    f.single(
        "alpha",
        r#", "peerDependencies":{"host":">=1 <3","beta":"^1"}"#,
    );
    f.single("beta", r#", "peerDependencies":{"host":"^1","alpha":"^1"}"#);
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file("package.json", r#"{"dependencies":{"alpha":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/host/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
    assert!(f.root.join("node_modules/beta/package.json").is_file());
    f.ok(&["ci", "--ignore-scripts"]);
    f.ok(&["ls", "--all"]);
}

#[test]
fn direct_peer_conflicts_warn_by_default_and_strict_force_legacy_are_explicit() {
    let f = Fixture::new();
    f.single(
        "@scope/plugin",
        r#", "peerDependencies":{"host":"^1 || ^3"}"#,
    );
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"@scope/plugin":"1","host":"2"}}"#,
    );
    let out = f.invoke(&["install", "--strict-peer-deps", "--ignore-scripts"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ERESOLVE"));
    assert!(!f.root.join("node_modules").exists());
    let out = f.invoke(&["install", "--ignore-scripts"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: overriding peer"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("host@^1 || ^3"));
    assert!(
        !f.root
            .join("node_modules/@scope/plugin/node_modules/host")
            .exists()
    );
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(!f.invoke(&["ci", "--strict-peer-deps"]).status.success());
    f.ok(&["ci", "--strict-peer-deps", "--force", "--ignore-scripts"]);
    f.ok(&["ci", "--legacy-peer-deps", "--ignore-scripts"]);
}

#[test]
fn root_peer_mismatches_warn_but_normal_dependency_ranges_are_not_relaxed() {
    let f = Fixture::new();
    f.releases("host", "7.0.0", &[("4.0.0", ""), ("7.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"host":"^4.0.0"},"peerDependencies":{"host":"^7.0.0"}}"#,
    );
    let output = f.invoke(&["install", "--ignore-scripts"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("root requires peer host@^7.0.0"));
    assert!(
        fs::read_to_string(f.root.join("node_modules/host/package.json"))
            .unwrap()
            .contains("4.0.0")
    );
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(
        !f.invoke(&["ci", "--strict-peer-deps", "--ignore-scripts"])
            .status
            .success()
    );
}

#[test]
fn missing_required_peers_still_fail_in_permissive_mode() {
    let f = Fixture::new();
    f.single("plugin", r#", "peerDependencies":{"host":"^4"}"#);
    f.releases("host", "7.0.0", &[("7.0.0", "")]);
    let original = r#"{"dependencies":{"plugin":"1"}}"#;
    f.file("package.json", original);
    for args in [
        &["install", "--ignore-scripts"][..],
        &["install", "--force", "--ignore-scripts"][..],
    ] {
        let output = f.invoke(args);
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(f.root.join("package.json")).unwrap(),
            original
        );
        assert!(!f.root.join("package-lock.json").exists());
        assert!(!f.root.join("node_modules").exists());
    }
}

#[test]
fn strict_peer_policy_can_still_be_selected_in_npmrc() {
    let f = Fixture::new();
    f.single("plugin", r#", "peerDependencies":{"host":"^4"}"#);
    f.releases("host", "7.0.0", &[("7.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"plugin":"1","host":"7"}}"#,
    );
    f.file(".npmrc", "strict-peer-deps=true\n");
    assert!(!f.invoke(&["install", "--ignore-scripts"]).status.success());
    assert!(!f.root.join("node_modules").exists());
    f.file(".npmrc", "strict-peer-deps=false\n");
    f.ok(&["install", "--ignore-scripts"]);
}

#[test]
fn optional_peer_is_checked_when_present_even_if_installed_later() {
    let f = Fixture::new();
    f.single(
        "aaa-plugin",
        r#", "peerDependencies":{"host":"^1"},"peerDependenciesMeta":{"host":{"optional":true}}"#,
    );
    f.single("zzz-provider", r#", "dependencies":{"host":"2"}"#);
    f.releases("host", "2.0.0", &[("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"aaa-plugin":"1","zzz-provider":"1"}}"#,
    );
    // The provider must keep host@2 nested rather than activating an incompatible
    // optional peer in the directly declared plugin's root peer environment.
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/host").exists());
    assert!(
        f.root
            .join("node_modules/zzz-provider/node_modules/host/package.json")
            .is_file()
    );
    f.ok(&["ci", "--ignore-scripts"]);
    f.file(
        "package.json",
        r#"{"dependencies":{"aaa-plugin":"1","host":"2"}}"#,
    );
    assert!(
        !f.invoke(&["install", "--strict-peer-deps", "--ignore-scripts"])
            .status
            .success()
    );
    let output = f.invoke(&["install", "--ignore-scripts"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("optional peer"));
}

#[test]
fn independent_peer_sets_are_nested_together_not_under_the_consumer() {
    let f = Fixture::new();
    f.single("a", r#", "dependencies":{"host":"1","plugin-one":"1"}"#);
    f.single("b", r#", "dependencies":{"host":"2","plugin-two":"1"}"#);
    f.single("plugin-one", r#", "peerDependencies":{"host":"1"}"#);
    f.single("plugin-two", r#", "peerDependencies":{"host":"2"}"#);
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file("package.json", r#"{"dependencies":{"a":"1","b":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        f.root
            .join("node_modules/b/node_modules/plugin-two/package.json")
            .is_file()
    );
    assert!(
        f.root
            .join("node_modules/b/node_modules/host/package.json")
            .is_file()
    );
    assert!(
        !f.root
            .join("node_modules/plugin-two/node_modules/host")
            .exists()
    );
    f.ok(&["ls", "--all"]);
    f.ok(&["ci", "--strict-peer-deps", "--ignore-scripts"]);
}

#[test]
fn peer_version_and_package_version_are_jointly_selected() {
    let f = Fixture::new();
    f.single("host", "");
    f.releases(
        "plugin",
        "2.0.0",
        &[
            ("1.0.0", r#", "peerDependencies":{"host":"1"}"#),
            ("2.0.0", r#", "peerDependencies":{"host":"2"}"#),
        ],
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"host":"1","plugin":"*"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/plugin/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
}

#[test]
fn ci_rejects_peers_nested_under_their_consumer_before_removal() {
    let f = Fixture::new();
    f.single("host", "");
    f.single("plugin", r#", "peerDependencies":{"host":"1"}"#);
    f.file("package.json", r#"{"dependencies":{"plugin":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    let path = f.root.join("tnpm-lock.json");
    let lock = fs::read_to_string(&path).unwrap().replace(
        "\"node_modules/host\"",
        "\"node_modules/plugin/node_modules/host\"",
    );
    fs::write(path, lock).unwrap();
    f.file("node_modules/sentinel", "keep");
    assert!(!f.invoke(&["ci", "--ignore-scripts"]).status.success());
    assert!(f.root.join("node_modules/sentinel").exists());
}

#[test]
fn transitive_overrides_warn_by_default_but_strict_mode_rejects_them() {
    let f = Fixture::new();
    f.single(
        "a",
        r#", "peerDependencies":{"host":"2"},"dependencies":{"plugin":"1"}"#,
    );
    f.single("plugin", r#", "peerDependencies":{"host":"1"}"#);
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file("package.json", r#"{"dependencies":{"a":"1","host":"2"}}"#);
    let out = f.invoke(&["install", "--ignore-scripts"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("overriding peer"));
    assert!(
        !f.invoke(&["update", "--strict-peer-deps", "--ignore-scripts"])
            .status
            .success()
    );
}

#[test]
fn omit_peer_excludes_only_peer_only_packages_and_does_not_change_the_lock() {
    let f = Fixture::new();
    f.single(
        "plugin",
        r#", "peerDependencies":{"host":"1","shared":"1"}"#,
    );
    f.single("host", r#", "dependencies":{"peer-child":"1"}"#);
    f.single("shared", "");
    f.single("peer-child", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"plugin":"1","shared":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    f.ok(&["ci", "--omit=peer", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/host").exists());
    assert!(!f.root.join("node_modules/peer-child").exists());
    assert!(f.root.join("node_modules/plugin/package.json").is_file());
    assert!(f.root.join("node_modules/shared/package.json").is_file());
    assert_eq!(
        fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap(),
        lock
    );
}
