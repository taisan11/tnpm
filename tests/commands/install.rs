//! CLI integration tests: install.
use crate::support::Fixture;
use std::fs;

#[test]
fn hoisting_does_not_shadow_an_already_satisfied_dependency() {
    let f = Fixture::new();
    f.single("app", r#", "dependencies":{"y":"1","z":"1"}"#);
    f.releases(
        "y",
        "2.0.0",
        &[
            ("1.0.0", r#", "dependencies":{"shared":"1"}"#),
            ("2.0.0", ""),
        ],
    );
    f.releases(
        "z",
        "2.0.0",
        &[
            ("1.0.0", r#", "dependencies":{"shared":"2"}"#),
            ("2.0.0", ""),
        ],
    );
    f.releases("shared", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file(
        "package.json",
        r#"{"dependencies":{"app":"1","shared":"1","y":"2","z":"2"}}"#,
    );
    f.ok(&["update", "--legacy-peer-deps", "--ignore-scripts"]);
    assert!(
        f.root
            .join("node_modules/app/node_modules/z/node_modules/shared/package.json")
            .is_file()
    );
    assert!(!f.root.join("node_modules/app/node_modules/shared").exists());
    f.ok(&["ci", "--legacy-peer-deps", "--ignore-scripts"]);
}

#[test]
fn packages_without_integrity_never_reuse_extracted_cache() {
    let f = Fixture::new();
    f.package("app", "1.0.0", r#", "description":"first""#);
    let first = f.routes.lock().unwrap()["/tar/app-1.0.0.tgz"].clone();
    f.package("app", "1.0.0", r#", "description":"second""#);
    let second = f.routes.lock().unwrap()["/tar/app-1.0.0.tgz"].clone();
    f.routes.lock().unwrap().insert("/first.tgz".into(), first);
    f.routes
        .lock()
        .unwrap()
        .insert("/second.tgz".into(), second);
    // A legacy cache entry without an integrity marker must not be trusted.
    f.file("home/.tnpm/cache/packages-v2/app@1.0.0/.complete", "");
    f.file(
        "home/.tnpm/cache/packages-v2/app@1.0.0/package/package.json",
        r#"{"name":"app","version":"1.0.0","description":"stale"}"#,
    );
    f.file("package.json", r#"{"dependencies":{"app":"1.0.0"}}"#);
    let lock = |tarball: &str| {
        format!(
            r#"{{"tnpmLockfileVersion":1,"packages":{{"":{{"dependencies":{{"app":"1.0.0"}}}},"node_modules/app":["1.0.0","{}{tarball}",null,{{}}]}}}}"#,
            f.url
        )
    };
    f.file("tnpm-lock.json", &lock("first.tgz"));
    f.ok(&["ci", "--ignore-scripts", "--no-save"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/app/package.json"))
            .unwrap()
            .contains("first")
    );
    f.file("tnpm-lock.json", &lock("second.tgz"));
    f.ok(&["ci", "--ignore-scripts", "--no-save"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/app/package.json"))
            .unwrap()
            .contains("second")
    );
    assert!(
        f.requests
            .lock()
            .unwrap()
            .contains(&"/second.tgz".to_owned())
    );
    assert!(
        !f.invoke(&["ci", "--ignore-scripts", "--no-save", "--offline"])
            .status
            .success()
    );
}

#[test]
fn install_lock_ci_bins_scripts_update_uninstall_and_prune() {
    let f = Fixture::new();
    let shared1 = f.package("shared", "1.0.0", "");
    let shared2 = f.package("shared", "2.0.0", "");
    let app = f.package(
        "app",
        "1.0.0",
        r#", "dependencies":{"shared":"^1"},"bin":{"app":"bin/cli.sh"}"#,
    );
    let scoped = f.package(
        "@scope/tool",
        "1.0.0",
        r#", "dependencies":{"shared":"^2"},"bin":"bin/cli.sh""#,
    );
    f.registry("shared", "2.0.0", &[("1.0.0", shared1), ("2.0.0", shared2)]);
    f.registry("app", "1.0.0", &[("1.0.0", app)]);
    f.registry("@scope/tool", "1.0.0", &[("1.0.0", scoped)]);
    f.file("package.json", r#"{"name":"fixture","version":"1.0.0","dependencies":{"app":"^1","@scope/tool":"^1"},"scripts":{"say":"app","presay":"echo pre","postsay":"echo post"}}"#);
    f.ok(&["i", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/app/package.json").is_file());
    assert!(
        f.root
            .join("node_modules/app/node_modules/shared/package.json")
            .is_file()
    );
    assert!(f.root.join("node_modules/shared/package.json").is_file());
    assert!(f.root.join("node_modules/.bin/tool").is_file());
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    assert!(lock.contains("\"tnpmLockfileVersion\": 1"));
    let before = f.requests.lock().unwrap().len();
    f.ok(&["install", "--ignore-scripts"]);
    assert_eq!(
        f.requests.lock().unwrap().len(),
        before,
        "warm install must not use network"
    );
    let output = f.ok(&[
        "run",
        "say",
        "--",
        "space value",
        "--json",
        "$(echo UNSAFE)",
    ]);
    assert_eq!(output, "pre\nspace value\n--json\n$(echo UNSAFE)\npost\n");
    assert_eq!(f.ok(&["x", "app", "--version"]), "--version\n");
    f.file(
        "node_modules/junk/package.json",
        r#"{"name":"junk","version":"1.0.0"}"#,
    );
    f.ok(&["prune"]);
    assert!(!f.root.join("node_modules/junk").exists());
    f.ok(&["ci", "--ignore-scripts"]);
    assert_eq!(
        fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap(),
        lock
    );
    assert_eq!(
        f.requests.lock().unwrap().len(),
        before,
        "ci must reuse extracted packages"
    );
    let newer = f.package(
        "app",
        "1.1.0",
        r#", "dependencies":{"shared":"^1"},"bin":{"app":"bin/cli.sh"}"#,
    );
    let original = f.package(
        "app",
        "1.0.0",
        r#", "dependencies":{"shared":"^1"},"bin":{"app":"bin/cli.sh"}"#,
    );
    f.registry("app", "1.1.0", &[("1.0.0", original), ("1.1.0", newer)]);
    f.ok(&["update", "app", "--no-cache", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/app/package.json"))
            .unwrap()
            .contains("1.1.0")
    );
    assert!(
        f.root
            .join("node_modules/app/node_modules/shared/package.json")
            .is_file()
    );
    f.ok(&["rm", "app", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/app").exists());
    assert!(!f.root.join("node_modules/.bin/app").exists());
    f.ok(&["ls", "--all"]);
}

#[test]
fn ci_rejects_inconsistent_lock_before_removing_modules() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"^1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.file("node_modules/sentinel", "keep");
    f.file("package.json", r#"{"dependencies":{"app":"^2"}}"#);
    assert!(!f.invoke(&["ci"]).status.success());
    assert!(f.root.join("node_modules/sentinel").exists());
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .all(|p| !p.contains("2.0.0"))
    );
}

#[test]
fn rejects_integrity_failure_and_unsafe_lock_paths() {
    let f = Fixture::new();
    let app = f.package("app", "1.0.0", "");
    f.registry(
        "app",
        "1.0.0",
        &[("1.0.0", app.replace("sha512-", "sha512-bad"))],
    );
    f.file("package.json", r#"{"dependencies":{"app":"1.0.0"}}"#);
    let out = f.invoke(&["install", "--ignore-scripts"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("EINTEGRITY"));
    assert!(!f.root.join("node_modules/app").exists());
    f.file("tnpm-lock.json", r#"{"tnpmLockfileVersion":1,"packages":{"":{"dependencies":{"app":"1.0.0"}},"node_modules/../../outside":["1.0.0",null,null,{}]}}"#);
    assert!(!f.invoke(&["ci"]).status.success());
}

#[test]
fn tarball_extraction_errors_keep_their_context_and_cause() {
    for optional in [false, true] {
        let f = Fixture::new();
        let bytes = tar::Builder::new(Vec::new()).into_inner().unwrap();
        f.routes.lock().unwrap().insert("/empty.tar".into(), bytes);
        let data = format!(
            r#"{{"name":"app","version":"1.0.0","dist":{{"tarball":"{}empty.tar"}}}}"#,
            f.url
        );
        f.registry("app", "1.0.0", &[("1.0.0", data)]);
        let field = if optional {
            "optionalDependencies"
        } else {
            "dependencies"
        };
        f.file("package.json", &format!(r#"{{"{field}":{{"app":"1"}}}}"#));

        let out = f.invoke(&["install", "--ignore-scripts"]);
        assert_eq!(out.status.success(), optional);
        let stderr = String::from_utf8(out.stderr).unwrap();
        for message in [
            "could not prepare node_modules/app",
            "could not unpack tarball",
            "tarball contains no package.json",
        ] {
            assert!(stderr.contains(message), "{stderr}");
        }
        if !optional {
            assert!(stderr.contains("  caused by: "), "{stderr}");
        }
        assert!(!f.root.join("node_modules/app").exists());
    }
}

#[test]
fn optional_platform_failures_and_shared_dev_dependencies() {
    let f = Fixture::new();
    let common = f.package("common", "1.0.0", "");
    let prod = f.package("prod", "1.0.0", r#", "dependencies":{"common":"1"}"#);
    let dev = f.package("dev", "1.0.0", r#", "dependencies":{"common":"1"}"#);
    let incompatible = f.package(
        "incompatible",
        "1.0.0",
        r#", "os":["never-an-os"],"dependencies":{"unique":"1"}"#,
    );
    let unique = f.package("unique", "1.0.0", "");
    for (name, data) in [
        ("common", common),
        ("prod", prod),
        ("dev", dev),
        ("incompatible", incompatible),
        ("unique", unique),
    ] {
        f.registry(name, "1.0.0", &[("1.0.0", data)]);
    }
    f.file("package.json", r#"{"dependencies":{"prod":"1"},"devDependencies":{"dev":"1"},"optionalDependencies":{"incompatible":"1","absent":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/incompatible").exists());
    assert!(!f.root.join("node_modules/unique").exists());
    f.ok(&["prune", "--production"]);
    assert!(!f.root.join("node_modules/dev").exists());
    assert!(f.root.join("node_modules/common/package.json").is_file());
    f.ok(&["ci", "--omit=dev", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/common/package.json").is_file());
}

#[test]
fn lock_only_and_no_save_do_not_modify_unrelated_state() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"name":"fixture"}"#);
    f.ok(&["i", "app", "--package-lock-only"]);
    assert_eq!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.as_str() == "/app")
            .count(),
        1,
        "explicit install should resolve metadata only once"
    );
    assert!(!f.root.join("node_modules").exists());
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    f.ok(&["install", "--ignore-scripts"]);
    f.ok(&["remove", "app", "--no-save", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/app").exists());
    assert_eq!(
        fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap(),
        lock
    );
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        manifest
    );
}

#[test]
fn stale_hidden_lock_is_not_trusted() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.file(
        "node_modules/app/package.json",
        r#"{"name":"app","version":"0.0.0"}"#,
    );
    f.file("node_modules/app/new-file", "changes directory mtime");
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/app/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
    f.file(
        "node_modules/extra/package.json",
        r#"{"name":"extra","version":"1.0.0"}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/extra").exists());
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_node_modules_and_runs_dependency_hooks() {
    let f = Fixture::new();
    let app = f.package(
        "app",
        "1.0.0",
        r#", "scripts":{"install":"echo built > built.txt"}"#,
    );
    f.registry("app", "1.0.0", &[("1.0.0", app)]);
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    fs::create_dir(f.root.join("outside")).unwrap();
    std::os::unix::fs::symlink(f.root.join("outside"), f.root.join("node_modules")).unwrap();
    assert!(!f.invoke(&["install"]).status.success());
    assert!(!f.root.join("outside/app").exists());
    fs::remove_file(f.root.join("node_modules")).unwrap();
    f.ok(&["install", "--trust", "app"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/built.txt")).unwrap(),
        "built\n"
    );
}

#[test]
fn offline_install_reuses_packages_and_never_falls_back_to_network() {
    let f = Fixture::new();
    f.file(".npmrc", "legacy-peer-deps=false\n");
    f.single("plugin", r#", "peerDependencies":{"host":"1"}"#);
    f.single("host", "");
    f.file("package.json", r#"{"dependencies":{"plugin":"1"}}"#);
    let before = f.requests.lock().unwrap().len();
    let out = f.invoke(&["install", "--offline", "--ignore-scripts"]);
    assert!(!out.status.success());
    assert_eq!(f.requests.lock().unwrap().len(), before);
    f.ok(&["install", "--ignore-scripts"]);
    let warm = f.requests.lock().unwrap().len();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    f.ok(&["install", "--offline", "--ignore-scripts"]);
    assert_eq!(f.requests.lock().unwrap().len(), warm);
    assert!(f.root.join("node_modules/host/package.json").is_file());
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    fs::remove_file(f.root.join("tnpm-lock.json")).unwrap();
    f.ok(&["install", "--offline", "--ignore-scripts"]);
    assert_eq!(f.requests.lock().unwrap().len(), warm);
}

#[test]
fn prune_all_resets_only_node_modules_and_supports_both_flags() {
    for flag in ["--all", "-a"] {
        let f = Fixture::new();
        f.single("app", "");
        f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
        f.ok(&["install", "--ignore-scripts"]);
        let manifest = fs::read(f.root.join("package.json")).unwrap();
        let lock = fs::read(f.root.join("tnpm-lock.json")).unwrap();
        let requests = f.requests.lock().unwrap().len();
        assert!(
            f.root
                .join("home/.tnpm/cache/packages-v2/app@1.0.0/package")
                .is_dir()
        );
        assert!(!f.root.join("home/.tnpm/cache/tarballs-v1").exists());
        f.ok(&["prune", flag]);
        assert!(!f.root.join("node_modules").exists());
        assert_eq!(fs::read(f.root.join("package.json")).unwrap(), manifest);
        assert_eq!(fs::read(f.root.join("tnpm-lock.json")).unwrap(), lock);
        f.ok(&["install", "--offline", "--ignore-scripts"]);
        assert_eq!(f.requests.lock().unwrap().len(), requests);
        assert!(f.root.join("node_modules/app/package.json").is_file());
    }
}

#[test]
fn package_cache_rejects_different_integrity_and_missing_cache_offline() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    let cache = f.root.join("home/.tnpm/cache/packages-v2/app@1.0.0");
    fs::write(cache.join(".complete"), "sha512-different").unwrap();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    let requests = f.requests.lock().unwrap().len();
    let out = f.invoke(&["install", "--offline", "--ignore-scripts"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ENOTCACHED"));
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.requests.lock().unwrap().len() > requests);
}

#[test]
fn npm_lockfiles_are_ignored_and_left_untouched() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file("package-lock.json", "not json");
    f.file("node_modules/.package-lock.json", "not json");
    let ci = f.invoke(&["ci", "--ignore-scripts"]);
    assert!(!ci.status.success());
    assert!(String::from_utf8_lossy(&ci.stderr).contains("ci requires tnpm-lock.json"));
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("tnpm-lock.json").is_file());
    assert!(f.root.join("node_modules/.tnpm-lock.json").is_file());
    assert_eq!(
        fs::read_to_string(f.root.join("package-lock.json")).unwrap(),
        "not json"
    );
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
}

#[test]
fn rejects_malformed_compact_locks_before_modifying_modules() {
    for entry in [
        r#"["1.0.0",null]"#,
        r#"[1,null,null,{}]"#,
        r#"["1.0.0",null,null,{"version":"9.0.0"}]"#,
    ] {
        let f = Fixture::new();
        f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
        f.file("node_modules/sentinel", "keep");
        f.file("tnpm-lock.json", &format!(r#"{{"tnpmLockfileVersion":1,"packages":{{"":{{"dependencies":{{"app":"1"}}}},"node_modules/app":{entry}}}}}"#));
        assert!(
            !f.invoke(&["ci", "--offline", "--ignore-scripts"])
                .status
                .success()
        );
        assert!(f.root.join("node_modules/sentinel").exists());
    }
}

#[test]
fn obsolete_native_lock_encodings_are_rejected() {
    let f = Fixture::new();
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file("node_modules/sentinel", "keep");
    for lock in [
        r#"{"tnpmLockfileVersion":1,"packages":{"":{"dependencies":{"app":"1"}},"node_modules/app":{"version":"1.0.0"}}}"#,
        r#"{"tnpmLockfileVersion":2,"packages":{"":{"dependencies":{"app":"1"}},"node_modules/app":["1.0.0",null,null,{}]}}"#,
    ] {
        f.file("tnpm-lock.json", lock);
        assert!(
            !f.invoke(&["ci", "--offline", "--ignore-scripts"])
                .status
                .success()
        );
        assert!(f.root.join("node_modules/sentinel").exists());
    }
}
