//! Isolated node_modules layout and shared extracted-store integration tests.
#![cfg(unix)]
use crate::support::Fixture;
use std::fs;

#[test]
fn isolated_layout_resolves_conflicting_transitive_versions_without_exposing_them() {
    let f = Fixture::new();
    f.releases("shared", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.single("left", r#", "dependencies":{"shared":"1"}"#);
    f.single("right", r#", "dependencies":{"shared":"2"}"#);
    f.file("package.json", r#"{"dependencies":{"left":"1","right":"1"}}"#);
    f.file(".npmrc", "node-linker=isolated\n");
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    assert!(!f.root.join("node_modules/shared").exists());
    for (name, expected) in [("left", "1.0.0"), ("right", "2.0.0")] {
        let path = f.root.join(format!("node_modules/{name}/node_modules/shared/package.json"));
        let data = fs::read_to_string(&path).unwrap();
        assert!(data.contains(&format!("\"version\":\"{expected}\"")), "{}", path.display());
    }
    f.ok(&["install", "--ignore-scripts", "--offline"]);
    f.ok(&["ci", "--ignore-scripts", "--offline"]);
    assert!(f.root.join("node_modules/left/node_modules/shared/package.json").is_file());
    f.ok(&["ls", "--all"]);
}

#[test]
fn isolated_linker_can_switch_to_hoisted_and_back() {
    let f = Fixture::new();
    f.single("app", r#", "dependencies":{"sub":"1"}"#);
    f.single("sub", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/sub").exists());
    f.ok(&["install", "--node-linker=hoisted", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/.tnpm-layout").exists());
    assert!(f.root.join("node_modules/sub/package.json").is_file());
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts", "--offline"]);
    assert!(f.root.join("node_modules/app/node_modules/sub/package.json").is_file());
}

#[test]
fn store_dir_and_user_configuration_are_shared_across_projects() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file("home/.npmrc", "node-linker=isolated\nstore-dir=~/shared-tnpm-store\n");
    let run = |args: &[&str]| {
        let output = f.command().args(args)
            .env_remove("npm_config_userconfig")
            .output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        output
    };
    let output = run(&["get", "node-linker"]);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "isolated\n");
    run(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    assert!(f.root.join("home/shared-tnpm-store/packages-v2/app@1.0.0/.complete").is_file());
    f.file(".npmrc", "node-linker=hoisted\n");
    assert_eq!(String::from_utf8(run(&["get", "node-linker"]).stdout).unwrap(), "hoisted\n");
    run(&["install", "--ignore-scripts", "--node-linker=isolated"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    let bad = f.invoke(&["install", "--node-linker=invalid"]);
    assert!(!bad.status.success());
}
