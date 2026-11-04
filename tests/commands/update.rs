//! CLI integration tests: update.
use crate::support::Fixture;
use std::fs;

#[test]
fn update_reports_before_after_versions_noop_and_single_json_document() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"^1.0.0"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.releases(
        "app",
        "2.0.0",
        &[("1.0.0", ""), ("1.1.0", ""), ("2.0.0", "")],
    );
    let output = f.ok(&["update", "--no-cache", "--ignore-scripts"]);
    for expected in [
        "Package",
        "Before",
        "After",
        "app",
        "1.0.0",
        "1.1.0",
        "Updated 1 package.",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert!(
        !output.contains("2.0.0"),
        "regular update must respect ^1: {output}"
    );
    assert_eq!(
        f.ok(&["update", "--no-cache", "--ignore-scripts"]),
        "No packages were updated.\n"
    );
    let output = f.ok(&[
        "update",
        "--latest",
        "--no-cache",
        "--ignore-scripts",
        "--json",
    ]);
    nojson::RawJson::parse(&output).unwrap();
    for expected in [
        "\"updated\": 1",
        "\"from\": \"1.1.0\"",
        "\"to\": \"2.0.0\"",
        "\"mode\": \"installed\"",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    let output = f.ok(&["update", "--json", "--ignore-scripts"]);
    nojson::RawJson::parse(&output).unwrap();
    assert!(output.contains("\"updated\": 0"));
}

#[test]
fn update_lock_only_reports_resolution_changes_without_claiming_installation() {
    let f = Fixture::new();
    f.releases("app", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file("package.json", r#"{"dependencies":{"app":"1.0.0"}}"#);
    f.ok(&["install", "--package-lock-only"]);
    let output = f.ok(&[
        "update",
        "--latest",
        "--package-lock-only",
        "--no-save",
        "--json",
    ]);
    nojson::RawJson::parse(&output).unwrap();
    assert!(output.contains("\"mode\": \"planned\""));
    assert!(!f.root.join("node_modules").exists());
    let output = f.ok(&["update", "--latest", "--package-lock-only", "--json"]);
    nojson::RawJson::parse(&output).unwrap();
    for expected in [
        "\"mode\": \"lockfile\"",
        "\"from\": \"1.0.0\"",
        "\"to\": \"2.0.0\"",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert!(!f.root.join("node_modules").exists());
}

#[test]
fn update_latest_upgrades_major_versions_preserving_fields_ranges_and_aliases() {
    let f = Fixture::new();
    for name in ["exact", "caret", "tilde", "real", "dev", "optional"] {
        f.releases(name, "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    }
    let original = r#"{"name":"fixture","custom":{"keep":true},"dependencies":{"exact":"1.0.0","caret":"^1.0.0","tilde":"~1.0.0","alias":"npm:real@^1.0.0"},"devDependencies":{"dev":"~1.0.0"},"optionalDependencies":{"optional":"^1.0.0"}}"#;
    f.file("package.json", original);
    f.ok(&["install", "--ignore-scripts"]);
    f.ok(&["update", "--latest", "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    for field in [
        "\"exact\": \"2.0.0\"",
        "\"caret\": \"^2.0.0\"",
        "\"tilde\": \"~2.0.0\"",
        "\"alias\": \"npm:real@^2.0.0\"",
        "\"dev\": \"~2.0.0\"",
        "\"optional\": \"^2.0.0\"",
        "\"keep\": true",
    ] {
        assert!(manifest.contains(field), "{manifest}");
    }
    for name in ["exact", "caret", "tilde", "alias", "dev", "optional"] {
        assert!(
            fs::read_to_string(f.root.join("node_modules").join(name).join("package.json"))
                .unwrap()
                .contains("2.0.0")
        );
    }
    assert!(!f.root.join("node_modules/real").exists());
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(f.ok(&["update", "--help"]).contains("--latest"));
}

#[test]
fn update_latest_targets_packages_and_respects_no_save_lock_only_and_exact() {
    let f = Fixture::new();
    for name in ["chosen", "untouched"] {
        f.releases(name, "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    }
    let original = r#"{"dependencies":{"chosen":"^1.0.0","untouched":"^1.0.0"}}"#;
    f.file("package.json", original);
    f.ok(&["install", "--package-lock-only"]);
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    f.ok(&[
        "update",
        "chosen",
        "--latest",
        "--no-save",
        "--package-lock-only",
    ]);
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap(),
        lock
    );
    assert!(!f.root.join("node_modules").exists());

    f.ok(&["update", "--latest", "chosen", "-E", "--package-lock-only"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains("\"chosen\": \"2.0.0\""));
    assert!(manifest.contains("\"untouched\": \"^1.0.0\""));
    assert!(!f.root.join("node_modules").exists());
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/chosen/package.json"))
            .unwrap()
            .contains("2.0.0")
    );
    assert!(
        fs::read_to_string(f.root.join("node_modules/untouched/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
}

#[test]
fn update_latest_upgrades_peer_sets_when_new_versions_are_compatible() {
    let f = Fixture::new();
    f.releases(
        "plugin",
        "2.0.0",
        &[
            ("1.0.0", r#", "peerDependencies":{"host":"^1"}"#),
            ("2.0.0", r#", "peerDependencies":{"host":"^2"}"#),
        ],
    );
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"plugin":"^1","host":"^1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    f.ok(&["update", "--latest", "--ignore-scripts"]);
    for name in ["plugin", "host"] {
        assert!(
            fs::read_to_string(f.root.join("node_modules").join(name).join("package.json"))
                .unwrap()
                .contains("2.0.0")
        );
    }
    f.ok(&["ci", "--ignore-scripts"]);
}

#[test]
fn update_latest_warns_by_default_and_strict_conflicts_do_not_mutate() {
    let f = Fixture::new();
    f.file(".npmrc", "legacy-peer-deps=false\n");
    f.single("plugin", r#", "peerDependencies":{"host":"^1"}"#);
    f.releases("host", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    let original = r#"{"dependencies":{"plugin":"^1.0.0","host":"^1.0.0"}}"#;
    f.file("package.json", original);
    f.ok(&["install", "--ignore-scripts"]);
    f.file("node_modules/.sentinel", "keep");
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    let output = f.invoke(&[
        "update",
        "--latest",
        "--strict-peer-deps",
        "--ignore-scripts",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("ERESOLVE"));
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap(),
        lock
    );
    assert!(f.root.join("node_modules/.sentinel").is_file());
    assert!(
        fs::read_to_string(f.root.join("node_modules/host/package.json"))
            .unwrap()
            .contains("1.0.0")
    );

    let output = f.invoke(&["update", "--latest", "--ignore-scripts"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("peer"));
    assert!(
        fs::read_to_string(f.root.join("node_modules/host/package.json"))
            .unwrap()
            .contains("2.0.0")
    );
    f.ok(&["ci", "--ignore-scripts"]);
}

#[test]
fn update_latest_rejects_undeclared_packages_and_non_registry_specs_before_writes() {
    let f = Fixture::new();
    let original = r#"{"dependencies":{"app":"file:../app"}}"#;
    f.file("package.json", original);
    for args in [
        &["update", "--latest"][..],
        &["update", "unknown", "--latest"][..],
        &["update", "app@next", "--latest"][..],
    ] {
        assert!(!f.invoke(args).status.success());
        assert_eq!(
            fs::read_to_string(f.root.join("package.json")).unwrap(),
            original
        );
        assert!(!f.root.join("package-lock.json").exists());
        assert!(!f.root.join("node_modules").exists());
    }
    assert!(f.requests.lock().unwrap().is_empty());
}
