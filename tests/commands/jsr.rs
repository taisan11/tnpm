use crate::support::Fixture;
use std::fs;

fn setup(f: &Fixture) {
    f.file(".npmrc", &format!("@jsr:registry={}\n", f.url));
    f.releases(
        "@jsr/demo__math",
        "2.0.0",
        &[("1.0.0", ""), ("1.1.0", ""), ("2.0.0", "")],
    );
}

#[test]
fn jsr_add_aliases_and_default_versions_preserve_native_specifiers() {
    let f = Fixture::new();
    setup(&f);
    f.ok(&["add", "jsr:@demo/math@^1", "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(
        manifest.contains("\"@demo/math\": \"jsr:@demo/math@^1\""),
        "{manifest}"
    );
    assert!(
        f.root
            .join("node_modules/@demo/math/package.json")
            .is_file()
    );
    assert!(!f.root.join("node_modules/@jsr/demo__math").exists());
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    assert!(lock.contains("@jsr/demo__math") && lock.contains("1.1.0"));
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
    f.ok(&["ls", "--all"]);
    f.ok(&["add", "alias@jsr:@demo/math@1", "-E", "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains("\"alias\": \"jsr:@demo/math@1.1.0\""));
    f.ok(&["add", "jsr:@demo/math", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("package.json"))
            .unwrap()
            .contains("jsr:@demo/math@^2.0.0")
    );
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .all(|path| !path.starts_with("/@demo%"))
    );
}

#[test]
fn jsr_outdated_update_and_view_use_the_compatibility_identity() {
    let f = Fixture::new();
    setup(&f);
    f.file(
        "package.json",
        r#"{"dependencies":{"@demo/math":"jsr:@demo/math@^1.0.0"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let outdated = f.invoke(&["outdated", "--json"]);
    assert_eq!(outdated.status.code(), Some(1));
    let text = String::from_utf8(outdated.stdout).unwrap();
    assert!(text.contains("@demo/math") && text.contains("2.0.0"));
    f.ok(&["update", "--latest", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("package.json"))
            .unwrap()
            .contains("jsr:@demo/math@^2.0.0")
    );
    f.ok(&["ci", "--ignore-scripts"]);
    assert_eq!(
        f.ok(&["view", "jsr:@demo/math@1", "version"]),
        "[\n  \"1.0.0\",\n  \"1.1.0\"\n]\n"
    );
}

#[test]
fn jsr_transitive_dependencies_and_peers_use_existing_resolution() {
    let f = Fixture::new();
    setup(&f);
    f.single("parent", r#", "dependencies":{"@demo/math":"jsr:@demo/math@^1"},"peerDependencies":{"@demo/math":"jsr:@demo/math@^1"}"#);
    f.file("package.json", r#"{"dependencies":{"parent":"1"}}"#);
    f.ok(&["install", "--strict-peer-deps", "--ignore-scripts"]);
    f.ok(&["ci", "--strict-peer-deps", "--ignore-scripts"]);
    f.ok(&["ls", "--all"]);
}

#[test]
fn malformed_jsr_specifiers_are_rejected_without_requests() {
    let f = Fixture::new();
    for spec in [
        "jsr:unscoped",
        "jsr:@demo/math@",
        "jsr:@demo/math@file:../x",
        "jsr:@demo/math/../secret",
        "jsr:@demo/math/subpath",
        "jsr:@demo/UPPER",
        "jsr:@demo/foo__bar",
    ] {
        assert!(!f.invoke(&["add", spec]).status.success(), "{spec}");
    }
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn jsr_has_a_builtin_registry_without_requiring_npmrc() {
    let f = Fixture::new();
    assert_eq!(f.ok(&["get", "@jsr:registry"]), "https://npm.jsr.io/\n");
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn jsr_registry_is_independent_of_the_npm_registry_override() {
    let f = Fixture::new();
    setup(&f);
    // A deliberately unreachable npm registry must not intercept jsr: metadata.
    let npm_registry = "http://127.0.0.1:1/";
    f.ok(&[
        "add",
        "jsr:@demo/math@^1",
        "--registry",
        npm_registry,
        "--ignore-scripts",
    ]);
    assert!(f.root.join("node_modules/@demo/math/package.json").is_file());
    assert_eq!(
        f.ok(&[
            "view",
            "jsr:@demo/math@1",
            "version",
            "--registry",
            npm_registry,
        ]),
        "[\n  \"1.0.0\",\n  \"1.1.0\"\n]\n"
    );
    assert!(f.requests.lock().unwrap().iter().any(|path| path.contains("@jsr")));
}

#[cfg(unix)]
#[test]
fn jsr_exec_uses_the_same_compatibility_mapping() {
    let f = Fixture::new();
    f.file(".npmrc", &format!("@jsr:registry={}\n", f.url));
    f.single("@jsr/demo__cli", r#", "bin":{"jsr-tool":"bin/cli.sh"}"#);
    assert_eq!(
        f.ok(&["exec", "jsr:@demo/cli@1", "forwarded"]),
        "forwarded\n"
    );
    assert!(!f.root.join("node_modules").exists());
}
