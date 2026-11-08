use crate::support::Fixture;
use std::fs;

fn script() -> &'static str {
    r#", "scripts":{"install":"echo built >> built.txt"}"#
}

#[test]
fn default_trust_is_extended_not_replaced_and_dependencies_cannot_self_authorize() {
    let f = Fixture::new();
    f.single("esbuild", script());
    f.single("custom", script());
    f.single(
        "untrusted",
        r#", "trustedDependencies":["untrusted"],"scripts":{"install":"echo bad > bad.txt"}"#,
    );
    f.file("package.json", r#"{"dependencies":{"esbuild":"1","custom":"1","untrusted":"1"},"trustedDependencies":["custom"],"scripts":{"install":"echo root > root.txt"}}"#);
    let out = f.invoke(&["install"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("blocked lifecycle scripts"));
    for name in ["esbuild", "custom"] {
        assert!(
            f.root
                .join(format!("node_modules/{name}/built.txt"))
                .is_file()
        );
    }
    assert!(!f.root.join("node_modules/untrusted/bad.txt").exists());
    assert!(f.root.join("root.txt").is_file());
    let requests = f.requests.lock().unwrap().len();
    f.ok(&["install", "--offline"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/custom/built.txt"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/esbuild/built.txt").exists());
    f.ok(&["install", "--offline"]);
    assert!(f.root.join("node_modules/esbuild/built.txt").is_file());
}

#[test]
fn empty_custom_list_keeps_defaults_and_alias_names_do_not_grant_trust() {
    let f = Fixture::new();
    f.single("esbuild", script());
    f.single("evil", script());
    f.file("package.json", r#"{"dependencies":{"safe-alias":"npm:esbuild@1","esbuild":"npm:evil@1"},"trustedDependencies":[]}"#);
    f.ok(&["install"]);
    assert!(f.root.join("node_modules/safe-alias/built.txt").is_file());
    assert!(!f.root.join("node_modules/esbuild/built.txt").exists());
}

#[test]
fn adding_trust_builds_previously_blocked_unchanged_packages() {
    let f = Fixture::new();
    f.single("custom", script());
    f.file("package.json", r#"{"dependencies":{"custom":"1"}}"#);
    f.ok(&["install"]);
    assert!(!f.root.join("node_modules/custom/built.txt").exists());
    f.file(
        "package.json",
        r#"{"dependencies":{"custom":"1"},"trustedDependencies":["custom"]}"#,
    );
    f.ok(&["install", "--offline"]);
    assert!(f.root.join("node_modules/custom/built.txt").is_file());
}

#[test]
fn an_archive_cannot_impersonate_a_trusted_manifest_name() {
    let f = Fixture::new();
    let metadata = f
        .package("esbuild", "1.0.0", script())
        .replace("\"name\":\"esbuild\"", "\"name\":\"evil\"");
    f.registry("evil", "1.0.0", &[("1.0.0", metadata)]);
    f.file("package.json", r#"{"dependencies":{"evil":"1"}}"#);
    f.ok(&["install"]);
    assert!(!f.root.join("node_modules/evil/built.txt").exists());
    f.ok(&["rebuild"]);
    assert!(!f.root.join("node_modules/evil/built.txt").exists());
}

#[cfg(unix)]
#[test]
fn rebuild_runs_only_trusted_implicit_native_builds() {
    let f = Fixture::new();
    f.file(
        "local/package.json",
        r#"{"name":"native","version":"1.0.0"}"#,
    );
    f.file("local/binding.gyp", "{}");
    f.file(
        "gyp.cjs",
        "require('fs').appendFileSync('built', 'built\\n');",
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"native":"file:local"}}"#,
    );
    f.ok(&["install"]);
    assert!(!f.root.join("node_modules/native/built").exists());
    let out = f
        .command()
        .args(["rebuild", "native", "--trust", "native", "--offline"])
        .env("npm_config_node_gyp", f.root.join("gyp.cjs"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/native/built")).unwrap(),
        "built\n"
    );
    f.ok(&["rebuild", "native", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/native/built")).unwrap(),
        "built\n"
    );
}

#[test]
fn successful_rebuild_does_not_cause_a_second_build_on_noop_install() {
    let f = Fixture::new();
    f.single("custom", script());
    f.file(
        "package.json",
        r#"{"dependencies":{"custom":"1"},"trustedDependencies":["custom"]}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    f.ok(&["rebuild", "custom"]);
    f.ok(&["install", "--offline"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/custom/built.txt"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn prepare_is_gated_for_source_dependencies_but_not_run_for_registry_packages() {
    let f = Fixture::new();
    f.single(
        "esbuild",
        r#", "scripts":{"prepare":"echo unexpected > prepared"}"#,
    );
    f.file(
        "local/package.json",
        r#"{"name":"custom","version":"1.0.0","scripts":{"prepare":"echo prepared > prepared"}}"#,
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"esbuild":"1","custom":"file:local"}}"#,
    );
    f.ok(&["install"]);
    assert!(!f.root.join("node_modules/custom/prepared").exists());
    assert!(!f.root.join("node_modules/esbuild/prepared").exists());
    f.ok(&["rebuild", "custom", "--trust", "custom"]);
    assert!(f.root.join("node_modules/custom/prepared").is_file());
    let json = f.ok(&["rebuild", "esbuild", "--json"]);
    assert!(json.contains("\"rebuilt\": 0"));
    assert!(!f.root.join("node_modules/esbuild/prepared").exists());
}

#[cfg(unix)]
#[test]
fn workspace_hooks_are_first_party_and_exec_inherits_only_project_trust() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"workspaces":["packages/*"],"trustedDependencies":["tool"]}"#,
    );
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","scripts":{"install":"echo built > built.txt"}}"#,
    );
    f.ok(&["install"]);
    assert!(f.root.join("packages/a/built.txt").is_file());
    f.single(
        "tool",
        r#", "bin":{"tool":"bin/cli.sh"},"scripts":{"install":"echo exec-built-marker"}"#,
    );
    let out = f.invoke(&["exec", "--json", "tool"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("exec-built-marker"));
}

#[test]
fn rebuild_is_offline_filtered_honors_trust_and_keeps_json_clean() {
    let f = Fixture::new();
    f.single("custom", script());
    f.single("esbuild", script());
    f.file(
        "package.json",
        r#"{"dependencies":{"custom":"1","esbuild":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let requests = f.requests.lock().unwrap().len();
    let lock = fs::read(f.root.join("tnpm-lock.json")).unwrap();
    f.ok(&["rebuild", "custom", "--offline"]);
    assert!(!f.root.join("node_modules/custom/built.txt").exists());
    let output = f.ok(&[
        "rebuild",
        "custom@1",
        "--trust",
        "custom",
        "--json",
        "--offline",
    ]);
    nojson::RawJson::parse(&output).unwrap();
    assert!(output.contains("\"rebuilt\": 1"));
    assert!(f.root.join("node_modules/custom/built.txt").is_file());
    assert!(!f.root.join("node_modules/esbuild/built.txt").exists());
    f.ok(&["rebuild", "node_modules/esbuild", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/esbuild/built.txt").exists());
    f.ok(&["rebuild", "--offline"]);
    assert!(f.root.join("node_modules/esbuild/built.txt").is_file());
    assert!(!f.invoke(&["rebuild", "missing"]).status.success());
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    assert_eq!(fs::read(f.root.join("tnpm-lock.json")).unwrap(), lock);
}

#[test]
fn malformed_trust_is_rejected_before_running_root_scripts() {
    let f = Fixture::new();
    for value in [r#""custom""#, r#"[42]"#, r#"["../custom"]"#] {
        f.file(
            "package.json",
            &format!(
                r#"{{"trustedDependencies":{value},"scripts":{{"preinstall":"echo bad > bad"}}}}"#
            ),
        );
        assert!(!f.invoke(&["install"]).status.success());
        assert!(!f.root.join("bad").exists());
    }
}

#[cfg(unix)]
#[test]
fn rebuild_rejects_symlink_escapes_and_preserves_script_exit_codes() {
    let f = Fixture::new();
    f.single("custom", r#", "scripts":{"install":"exit 7"}"#);
    f.file(
        "package.json",
        r#"{"dependencies":{"custom":"1"},"trustedDependencies":["custom"]}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert_eq!(f.invoke(&["rebuild", "custom"]).status.code(), Some(7));
    fs::rename(f.root.join("node_modules/custom"), f.root.join("outside")).unwrap();
    std::os::unix::fs::symlink(f.root.join("outside"), f.root.join("node_modules/custom")).unwrap();
    let out = f.invoke(&["rebuild"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("symlinked installation path"));
}
