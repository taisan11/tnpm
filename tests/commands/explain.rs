use crate::support::Fixture;
use std::fs;

#[test]
fn why_explains_shared_dependencies_peers_and_development_edges_without_modules() {
    let f = Fixture::new();
    f.single(
        "a",
        r#", "dependencies":{"shared":"1"},"peerDependencies":{"peer":"1"}"#,
    );
    f.single("b", r#", "dependencies":{"shared":"1"}"#);
    f.single("shared", "");
    f.single("peer", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"a":"1"},"devDependencies":{"b":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let before = fs::read(f.root.join("tnpm-lock.json")).unwrap();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    let requests = f.requests.lock().unwrap().len();
    let text = f.ok(&["why", "shared"]);
    assert!(
        text.contains("root -> node_modules/a -> node_modules/shared"),
        "{text}"
    );
    assert!(text.contains("node_modules/a requires shared@1 [prod]"));
    assert!(text.contains("node_modules/b requires shared@1 [prod]"));
    let json = f.ok(&["explain", "peer", "b", "--json"]);
    nojson::RawJson::parse(&json).unwrap();
    assert!(json.contains("\"type\": \"peer\""));
    assert!(json.contains("\"type\": \"dev\""));
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    assert_eq!(fs::read(f.root.join("tnpm-lock.json")).unwrap(), before);
}

#[test]
fn explain_distinguishes_nested_versions_paths_aliases_and_cycles() {
    let f = Fixture::new();
    f.single("a", r#", "dependencies":{"shared":"1"}"#);
    f.single("b", r#", "dependencies":{"shared":"2"}"#);
    f.releases(
        "shared",
        "2.0.0",
        &[("1.0.0", r#", "dependencies":{"a":"1"}"#), ("2.0.0", "")],
    );
    f.single("real", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"a":"1","b":"1","alias":"npm:real@1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let json = f.ok(&["why", "shared@2", "--json"]);
    assert!(json.contains("2.0.0") && !json.contains("\"version\": \"1.0.0\""));
    assert!(
        f.ok(&["why", "node_modules/b/node_modules/shared"])
            .contains("shared@2.0.0")
    );
    assert!(f.ok(&["why", "alias"]).contains("real@1.0.0"));
    assert!(f.ok(&["why", "real"]).contains("node_modules/alias"));
    assert!(!f.invoke(&["why", "absent"]).status.success());
    assert!(!f.invoke(&["why"]).status.success());
}
