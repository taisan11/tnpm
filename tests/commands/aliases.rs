//! CLI integration tests: aliases.
use crate::support::Fixture;
use std::fs;

#[test]
fn npm_aliases_use_target_scoped_registry_for_outdated_install_update_and_ci() {
    let f = Fixture::new();
    let target = "@jsr/taisan11__vite-plugin-budoux-build";
    let alias = "@taisan11/vite-plugin-budoux-build";
    let one = f.package(target, "1.0.0", "");
    let wanted = f.package(target, "1.1.0", "");
    let latest = f.package(target, "2.0.0", "");
    f.registry(
        target,
        "2.0.0",
        &[("1.0.0", one), ("1.1.0", wanted), ("2.0.0", latest)],
    );
    let metadata_path = format!("/{}", target.replace('/', "%2F"));
    let scoped_path = format!("/jsr{metadata_path}");
    {
        let mut routes = f.routes.lock().unwrap();
        let metadata = routes.remove(&metadata_path).unwrap();
        routes.insert(scoped_path.clone(), metadata);
    }
    f.file(
        ".npmrc",
        &format!(
            "@jsr:registry={}jsr/\n//{}jsr/:_authToken=${{TNPM_TEST_TOKEN}}\n",
            f.url,
            f.url.strip_prefix("http://").unwrap()
        ),
    );
    let manifest = format!(r#"{{"dependencies":{{"{alias}":"npm:{target}@^1.0.1"}}}}"#);
    f.file("package.json", &manifest);
    let invoke = |args: &[&str]| {
        f.command()
            .args(args)
            .env("TNPM_TEST_TOKEN", "fixture-token")
            .output()
            .unwrap()
    };

    let output = invoke(&["outdated"]);
    assert_eq!(output.status.code(), Some(1));
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(!table.contains('\t'));
    assert!(table.contains(alias), "{table}");
    assert!(table.contains("MISSING"), "{table}");
    assert!(table.contains("1.1.0"), "{table}");
    assert!(table.contains("2.0.0"), "{table}");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("unsupported"));
    let output = invoke(&["outdated", "--json"]);
    assert_eq!(output.status.code(), Some(1));
    let json = String::from_utf8(output.stdout).unwrap();
    assert!(json.contains(&format!("\"{alias}\"")));
    assert!(json.contains("\"wanted\": \"1.1.0\""));
    assert!(json.contains("\"latest\": \"2.0.0\""));
    for args in [
        &["install", "--ignore-scripts"][..],
        &["update", alias, "--ignore-scripts"][..],
        &["ci", "--ignore-scripts"][..],
    ] {
        let output = invoke(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        f.root
            .join("node_modules")
            .join(alias)
            .join("package.json")
            .is_file()
    );
    assert!(!f.root.join("node_modules").join(target).exists());
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        manifest
    );
    let lock = fs::read_to_string(f.root.join("package-lock.json")).unwrap();
    assert!(lock.contains(&format!("\"name\": \"{target}\"")));
    assert!(f.ok(&["ls", "--json"]).contains(alias));

    f.file("node_modules/sentinel", "keep");
    f.file(
        "package-lock.json",
        &lock.replace(
            &format!("\"name\": \"{target}\""),
            "\"name\": \"wrong-target\"",
        ),
    );
    let output = invoke(&["ci", "--ignore-scripts"]);
    assert!(!output.status.success());
    assert!(f.root.join("node_modules/sentinel").is_file());
    let requests = f.authorizations.lock().unwrap();
    assert!(requests.iter().any(|(path, _)| path == &scoped_path));
    for (path, auth) in requests.iter() {
        if path == &scoped_path {
            assert_eq!(auth.as_deref(), Some("Bearer fixture-token"));
        } else {
            assert!(auth.is_none(), "credentials must not be sent outside /jsr/");
        }
    }
}

#[test]
fn installing_an_alias_preserves_its_spec_and_real_package_identity() {
    let f = Fixture::new();
    f.single("real", "");
    f.file("package.json", "{}");
    f.ok(&["install", "alias@npm:real@^1", "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains("\"alias\": \"npm:real@^1\""));
    f.ok(&["ci", "--ignore-scripts"]);
    f.ok(&["install", "exact@npm:real@latest", "-E", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("package.json"))
            .unwrap()
            .contains("\"exact\": \"npm:real@1.0.0\"")
    );
    assert!(f.root.join("node_modules/alias/package.json").is_file());
    assert!(f.root.join("node_modules/exact/package.json").is_file());
}

#[test]
fn transitive_aliases_with_different_targets_are_not_deduplicated_by_version() {
    let f = Fixture::new();
    f.single("one", "");
    f.single("two", "");
    f.single(
        "app",
        r#", "dependencies":{"alias":"npm:two@^1"},"peerDependencies":{"peer":"1"}"#,
    );
    f.single("peer", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"alias":"npm:one@^1","app":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    for (path, target) in [
        ("node_modules/alias/package.json", "one"),
        ("node_modules/app/node_modules/alias/package.json", "two"),
    ] {
        assert!(
            fs::read_to_string(f.root.join(path))
                .unwrap()
                .contains(&format!("\"name\":\"{target}\""))
        );
    }
    f.ok(&["ci", "--ignore-scripts"]);
    f.ok(&["ls", "--all", "--json"]);
}
