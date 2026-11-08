use crate::support::Fixture;
use std::{fs, process::Command};

fn failed(f: &Fixture, args: &[&str], message: &str) {
    let out = f.invoke(args);
    assert!(!out.status.success(), "unexpected success: {args:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(message),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn local_directory_and_archive_install_without_registry_and_refresh() {
    let f = Fixture::new();
    f.file(
        "local/package.json",
        r#"{"name":"local-pkg","version":"1.0.0"}"#,
    );
    f.file("local/index.js", "first");
    f.ok(&["add", "./local", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/local-pkg/index.js")).unwrap(),
        "first"
    );
    f.file("local/index.js", "second");
    f.ok(&["install", "--offline", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/local-pkg/index.js")).unwrap(),
        "second"
    );
    assert!(f.requests.lock().unwrap().is_empty());
    f.ok(&["outdated", "--offline"]);
    f.package("archive-pkg", "1.0.0", "");
    let bytes = f.routes.lock().unwrap()["/tar/archive-pkg-1.0.0.tgz"].clone();
    fs::write(f.root.join("archive.tgz"), bytes).unwrap();
    f.ok(&["add", "./archive.tgz", "--ignore-scripts"]);
    assert!(
        f.root
            .join("node_modules/archive-pkg/package.json")
            .is_file()
    );
}

#[test]
fn local_transitive_paths_resolve_relative_to_the_source() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"dependencies":{"a":"file:packages/a"}}"#,
    );
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","dependencies":{"b":"file:../b"}}"#,
    );
    f.file(
        "packages/b/package.json",
        r#"{"name":"b","version":"1.0.0"}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/b/package.json").is_file());
    assert!(f.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn workspaces_link_packages_install_dev_dependencies_and_run_selected_scripts() {
    let f = Fixture::new();
    f.single("tool", "");
    f.file(
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    f.file("packages/a/package.json", r#"{"name":"a","version":"1.0.0","dependencies":{"b":"workspace:^"},"devDependencies":{"tool":"1"},"scripts":{"check":"printf a > result"}}"#);
    f.file(
        "packages/b/package.json",
        r#"{"name":"b","version":"1.1.0","scripts":{"check":"printf b > result"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::symlink_metadata(f.root.join("node_modules/a"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::canonicalize(f.root.join("node_modules/b")).unwrap(),
        fs::canonicalize(f.root.join("packages/b")).unwrap()
    );
    assert!(f.root.join("node_modules/tool/package.json").is_file());
    f.ok(&["run", "-w", "a", "check"]);
    assert_eq!(
        fs::read_to_string(f.root.join("packages/a/result")).unwrap(),
        "a"
    );
    assert!(!f.root.join("packages/b/result").exists());
    f.ok(&["run", "--workspaces", "check"]);
    f.ok(&["install", "--offline", "--ignore-scripts"]);
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
    f.ok(&["prune"]);
    assert!(f.root.join("node_modules/a/package.json").is_file());
}

#[cfg(unix)]
#[test]
fn workspace_globs_match_character_classes_recursive_paths_and_exclusions() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"workspaces":["packages/**/item-[0-9]","packages/extra-?/unit?","!packages/skip/**"]}"#,
    );
    f.file(
        "packages/a/nested/item-1/package.json",
        r#"{"name":"included","version":"1.0.0"}"#,
    );
    f.file(
        "packages/skip/deep/item-2/package.json",
        r#"{"name":"excluded","version":"1.0.0"}"#,
    );
    f.file(
        "packages/a/nested/item-x/package.json",
        r#"{"name":"not-matched","version":"1.0.0"}"#,
    );
    f.file(
        "packages/extra-x/unit4/package.json",
        r#"{"name":"extra","version":"1.0.0"}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/included/package.json").is_file());
    assert!(f.root.join("node_modules/extra/package.json").is_file());
    assert!(!f.root.join("node_modules/excluded").exists());
    assert!(!f.root.join("node_modules/not-matched").exists());
}

#[cfg(unix)]
#[test]
fn invalid_workspace_glob_is_rejected_before_fetching_dependencies() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/[a-"]}"#);
    assert!(!f.invoke(&["install"]).status.success());
    assert!(f.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn workspace_add_and_remove_edit_only_the_selected_manifest() {
    let f = Fixture::new();
    f.single("tool", "");
    f.file(
        "package.json",
        r#"{"name":"root","workspaces":["packages/*"]}"#,
    );
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    f.ok(&["add", "-w", "a", "tool", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("packages/a/package.json"))
            .unwrap()
            .contains("tool")
    );
    assert!(
        !fs::read_to_string(f.root.join("package.json"))
            .unwrap()
            .contains("tool")
    );
    f.ok(&["uninstall", "-w", "a", "tool", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/tool").exists());
    f.file(
        "helper/package.json",
        r#"{"name":"helper","version":"1.0.0"}"#,
    );
    f.ok(&["add", "-w", "a", "./helper", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("packages/a/package.json"))
            .unwrap()
            .contains("../../helper")
    );
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
}

#[test]
fn overrides_replace_transitive_versions_and_dollar_references() {
    let f = Fixture::new();
    f.single("parent", r#", "dependencies":{"child":"1"}"#);
    f.releases("child", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"parent":"1"},"overrides":{"child":"2"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let data = fs::read_to_string(f.root.join("node_modules/child/package.json")).unwrap();
    assert!(data.contains("2.0.0"), "{data}");
    f.ok(&["ci", "--ignore-scripts"]);
    f.ok(&["ls", "--all"]);
    f.file(
        "package.json",
        r#"{"dependencies":{"parent":"1","child":"1"},"overrides":{"child":"$child"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/child/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"child":"1"},"overrides":{"child":"2"}}"#,
    );
    failed(&f, &["install"], "EOVERRIDE");
}

#[test]
fn nested_overrides_remain_scoped_after_hoisting_and_peer_search() {
    let f = Fixture::new();
    f.single("a", r#", "dependencies":{"middle":"1"}"#);
    f.single("b", r#", "dependencies":{"middle":"1"}"#);
    f.single(
        "middle",
        r#", "dependencies":{"child":"1"},"peerDependencies":{"peer":"1"}"#,
    );
    f.single("peer", "");
    f.releases("child", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"a":"1","b":"1"},"overrides":{"a@1":{"child":"2"}}}"#,
    );
    f.ok(&["install", "--strict-peer-deps", "--ignore-scripts"]);
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    assert!(lock.contains("1.0.0") && lock.contains("2.0.0"));
    assert_eq!(lock.matches("node_modules/child\"").count(), 2);
    f.ok(&["ci", "--strict-peer-deps", "--ignore-scripts"]);
    f.ok(&["ls", "--all"]);
}

#[test]
fn engine_strict_fails_before_removing_existing_modules() {
    let f = Fixture::new();
    f.single("bad-engine", r#", "engines":{"node":"<0.0.1"}"#);
    f.file("package.json", r#"{"dependencies":{"bad-engine":"1"}}"#);
    f.file("node_modules/keep.txt", "keep");
    failed(
        &f,
        &["install", "--engine-strict", "--ignore-scripts"],
        "EBADENGINE",
    );
    assert!(f.root.join("node_modules/keep.txt").is_file());
    f.ok(&["install", "--ignore-scripts"]);
    f.ok(&["install", "--engine-strict", "--force", "--ignore-scripts"]);
}

#[cfg(target_os = "linux")]
#[test]
fn libc_mismatch_rejects_required_but_skips_optional_packages() {
    let f = Fixture::new();
    f.single("wrong-libc", r#", "libc":["impossible-libc"]"#);
    f.file("package.json", r#"{"dependencies":{"wrong-libc":"1"}}"#);
    failed(&f, &["install", "--ignore-scripts"], "EBADPLATFORM");
    f.file(
        "package.json",
        r#"{"optionalDependencies":{"wrong-libc":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/wrong-libc").exists());
}

#[cfg(unix)]
#[test]
fn native_default_build_respects_gypfile_and_ignore_scripts() {
    let f = Fixture::new();
    f.file(
        "local/package.json",
        r#"{"name":"native","version":"1.0.0"}"#,
    );
    f.file("local/binding.gyp", "{}");
    f.file(
        "fake-gyp.js",
        "require('fs').writeFileSync('built', process.argv[2]);",
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"native":"file:local"}}"#,
    );
    let out = f
        .command()
        .args(["install", "--trust", "native"])
        .env("npm_config_node_gyp", f.root.join("fake-gyp.js"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/native/built")).unwrap(),
        "rebuild"
    );
    f.ok(&["ci", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/native/built").exists());
    f.file(
        "local/package.json",
        r#"{"name":"native","version":"1.0.0","gypfile":false}"#,
    );
    f.ok(&["install"]);
    assert!(!f.root.join("node_modules/native/built").exists());
}

fn git(f: &Fixture, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(f.root.join("repo"))
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

#[test]
fn git_dependencies_pin_commits_and_reinstall_offline() {
    let f = Fixture::new();
    f.file(
        "repo/package.json",
        r#"{"name":"git-pkg","version":"1.0.0"}"#,
    );
    f.file("repo/index.js", "first");
    git(&f, &["init", "--quiet"]);
    git(&f, &["add", "."]);
    git(&f, &["commit", "--quiet", "-m", "initial"]);
    let commit = git(&f, &["rev-parse", "HEAD"]);
    let url = reqwest::Url::from_file_path(f.root.join("repo")).unwrap();
    f.file(
        "package.json",
        &format!(r#"{{"dependencies":{{"git-pkg":"git+{url}#{commit}"}}}}"#),
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("tnpm-lock.json"))
            .unwrap()
            .contains(&commit)
    );
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/git-pkg/index.js")).unwrap(),
        "first"
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn global_install_creates_bins_uninstall_preserves_unrelated_files() {
    let f = Fixture::new();
    f.single("tool", r#", "bin":{"my-tool":"bin/cli.sh"}"#);
    f.ok(&["install", "-g", "tool", "--ignore-scripts"]);
    let prefix = f.root.join("home/.tnpm/global");
    assert!(prefix.join("lib/node_modules/tool/package.json").is_file());
    let output = Command::new(prefix.join("bin/my-tool"))
        .arg("hello")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "hello\n");
    f.file("home/.tnpm/global/bin/unrelated", "keep");
    f.ok(&["ls", "-g"]);
    f.ok(&["uninstall", "-g", "tool", "--ignore-scripts"]);
    assert!(!prefix.join("bin/my-tool").exists());
    assert!(prefix.join("bin/unrelated").is_file());
}

#[cfg(unix)]
#[test]
fn global_install_refuses_unowned_binary_collision() {
    let f = Fixture::new();
    f.single("tool", r#", "bin":{"my-tool":"bin/cli.sh"}"#);
    f.file("home/.tnpm/global/bin/my-tool", "keep");
    failed(
        &f,
        &["install", "-g", "tool", "--ignore-scripts"],
        "unowned global binary",
    );
    assert_eq!(
        fs::read_to_string(f.root.join("home/.tnpm/global/bin/my-tool")).unwrap(),
        "keep"
    );
}

#[cfg(unix)]
#[test]
fn global_prefix_preserves_packages_and_manifests_it_does_not_own() {
    let f = Fixture::new();
    f.single("tool", "");
    f.file(
        "home/.tnpm/global/lib/node_modules/unmanaged/package.json",
        r#"{"name":"unmanaged","version":"1.0.0"}"#,
    );
    failed(&f, &["add", "-g", "tool"], "unmanaged global package");
    assert!(
        f.root
            .join("home/.tnpm/global/lib/node_modules/unmanaged/package.json")
            .is_file()
    );
    f.file("custom/lib/package.json", r#"{"name":"unrelated"}"#);
    let prefix = f.root.join("custom");
    failed(
        &f,
        &["add", "-g", "--prefix", prefix.to_str().unwrap(), "tool"],
        "unrelated global manifest",
    );
    assert_eq!(
        fs::read_to_string(f.root.join("custom/lib/package.json")).unwrap(),
        r#"{"name":"unrelated"}"#
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn qualified_self_overrides_carry_child_rules_to_the_replacement_version() {
    let f = Fixture::new();
    f.single("root-dep", r#", "dependencies":{"parent":"1"}"#);
    f.releases(
        "parent",
        "2.0.0",
        &[
            ("1.0.0", r#", "dependencies":{"child":"1"}"#),
            ("2.0.0", r#", "dependencies":{"child":"1"}"#),
        ],
    );
    f.releases("child", "3.0.0", &[("1.0.0", ""), ("3.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"root-dep":"1"},"overrides":{"parent@1":{".":"2","child":"3"}}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/parent/package.json"))
            .unwrap()
            .contains("2.0.0")
    );
    assert!(
        fs::read_to_string(f.root.join("node_modules/child/package.json"))
            .unwrap()
            .contains("3.0.0")
    );
    f.ok(&["ls", "--all"]);
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
}

#[test]
fn bundled_dependencies_are_locked_linked_and_reused_offline() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha512};
    let f = Fixture::new();
    f.single("leaf", "");
    f.single("peer", "");
    let mut archive = tar::Builder::new(Vec::new());
    for (path, contents) in [
        (
            "package/package.json",
            r#"{"name":"parent","version":"1.0.0","dependencies":{"bundled":"1"},"bundleDependencies":["bundled"],"peerDependencies":{"peer":"1"}}"#,
        ),
        (
            "package/node_modules/bundled/package.json",
            r#"{"name":"bundled","version":"1.0.0","dependencies":{"leaf":"1"},"bin":{"bundled":"cli.js"},"scripts":{"install":"node -e \"require('fs').writeFileSync('installed','yes')\""}}"#,
        ),
        (
            "package/node_modules/bundled/cli.js",
            "#!/usr/bin/env node\nconsole.log('bundled');\n",
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, path, contents.as_bytes())
            .unwrap();
    }
    let bytes = archive.into_inner().unwrap();
    let integrity = STANDARD.encode(Sha512::digest(&bytes));
    f.routes.lock().unwrap().insert("/parent.tar".into(), bytes);
    f.registry("parent", "1.0.0", &[("1.0.0", format!(r#"{{"name":"parent","version":"1.0.0","dependencies":{{"bundled":"1"}},"bundleDependencies":["bundled"],"peerDependencies":{{"peer":"1"}},"dist":{{"tarball":"{}parent.tar","integrity":"sha512-{integrity}"}}}}"#, f.url))]);
    f.file("package.json", r#"{"dependencies":{"parent":"1"}}"#);
    f.ok(&["install", "--strict-peer-deps", "--trust", "bundled"]);
    let bundled = f.root.join("node_modules/parent/node_modules/bundled");
    assert_eq!(
        fs::read_to_string(bundled.join("installed")).unwrap(),
        "yes"
    );
    assert!(f.root.join("node_modules/leaf/package.json").is_file());
    assert!(
        fs::read_to_string(f.root.join("tnpm-lock.json"))
            .unwrap()
            .contains("tnpmBundlePath")
    );
    assert!(!f.requests.lock().unwrap().iter().any(|p| p == "/bundled"));
    let requests = f.requests.lock().unwrap().len();
    f.ok(&["ci", "--offline", "--strict-peer-deps", "--ignore-scripts"]);
    assert!(bundled.join("cli.js").is_file());
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    f.ok(&["prune"]);
    assert!(bundled.join("cli.js").is_file());
}

#[cfg(unix)]
#[test]
fn workspace_dev_omit_and_nested_conflicts_do_not_modify_unrelated_paths() {
    let f = Fixture::new();
    f.releases("shared", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.single("dev-tool", "");
    f.file(
        "package.json",
        r#"{"workspaces":["packages/*"],"dependencies":{"shared":"1"}}"#,
    );
    f.file("packages/a/package.json", r#"{"name":"a","version":"1.0.0","dependencies":{"shared":"2"},"devDependencies":{"dev-tool":"1"}}"#);
    f.ok(&["install", "--omit=dev", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/dev-tool").exists());
    assert!(
        fs::read_to_string(f.root.join("packages/a/node_modules/shared/package.json"))
            .unwrap()
            .contains("2.0.0")
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/dev-tool/package.json").is_file());
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0","dependencies":{"shared":"1"}}"#,
    );
    f.file(
        "node_modules/keep/package.json",
        r#"{"name":"keep","version":"1.0.0"}"#,
    );
    failed(
        &f,
        &["ci", "--ignore-scripts"],
        "source package.json and lockfile disagree",
    );
    assert!(f.root.join("node_modules/keep/package.json").is_file());
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("packages/a/node_modules/shared").exists());
}

#[cfg(unix)]
#[test]
fn unexpected_workspace_links_and_local_symlinks_are_rejected() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/*"]}"#);
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","version":"1.0.0"}"#,
    );
    f.file(
        "outside/package.json",
        r#"{"name":"outside","version":"1.0.0"}"#,
    );
    f.file("outside/keep", "keep");
    f.ok(&["install", "--ignore-scripts"]);
    fs::remove_file(f.root.join("node_modules/a")).unwrap();
    std::os::unix::fs::symlink(f.root.join("outside"), f.root.join("node_modules/a")).unwrap();
    failed(&f, &["install", "--ignore-scripts"], "unexpected target");
    assert_eq!(
        fs::read_to_string(f.root.join("outside/keep")).unwrap(),
        "keep"
    );
    f.file("package.json", r#"{"dependencies":{"local":"file:local"}}"#);
    f.file(
        "local/package.json",
        r#"{"name":"local","version":"1.0.0"}"#,
    );
    std::os::unix::fs::symlink(f.root.join("outside"), f.root.join("local/escape")).unwrap();
    // Use a fresh tree so this assertion checks source copying, not the prior bad link.
    fs::remove_file(f.root.join("node_modules/a")).unwrap();
    failed(&f, &["install", "--ignore-scripts"], "contains symlink");
}

#[test]
fn git_branch_is_pinned_until_update_and_semver_tags_are_supported() {
    let f = Fixture::new();
    f.file(
        "repo/package.json",
        r#"{"name":"git-pkg","version":"1.0.0"}"#,
    );
    f.file("repo/index.js", "first");
    git(&f, &["init", "--quiet"]);
    git(&f, &["add", "."]);
    git(&f, &["commit", "--quiet", "-m", "initial"]);
    git(&f, &["tag", "v1.0.0"]);
    let branch = git(&f, &["branch", "--show-current"]);
    let url = reqwest::Url::from_file_path(f.root.join("repo")).unwrap();
    f.file(
        "package.json",
        &format!(r#"{{"dependencies":{{"git-pkg":"git+{url}#{branch}"}}}}"#),
    );
    f.ok(&["install", "--ignore-scripts"]);
    f.file("repo/index.js", "second");
    git(&f, &["add", "."]);
    git(&f, &["commit", "--quiet", "-m", "second"]);
    git(&f, &["tag", "v1.1.0"]);
    f.ok(&["install", "--offline", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/git-pkg/index.js")).unwrap(),
        "first"
    );
    f.ok(&["update", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/git-pkg/index.js")).unwrap(),
        "second"
    );
    f.file(
        "package.json",
        &format!(r#"{{"dependencies":{{"git-pkg":"git+{url}#semver:^1.0.0"}}}}"#),
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/git-pkg/index.js")).unwrap(),
        "second"
    );
}
