//! CLI integration tests: project.
use crate::support::Fixture;
use std::fs;

#[test]
fn init_paths_are_local_and_create_never_falls_back_to_init() {
    let f = Fixture::new();
    let parent = r#"{"name":"parent","custom":true}"#;
    f.file("package.json", parent);
    let output = f.ok(&["init", "-y", "--json"]);
    assert!(output.contains("\"version\": \"1.0.0\""));
    assert!(output.contains("\"license\": \"ISC\""));
    let parent = fs::read_to_string(f.root.join("package.json")).unwrap();
    f.ok(&["init", "vite", "-y", "--json"]);
    assert!(
        fs::read_to_string(f.root.join("vite/package.json"))
            .unwrap()
            .contains("\"name\": \"vite\"")
    );
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        parent
    );
    f.file(
        "vite/package.json",
        r#"{"name":"keep","version":"2.0.0","custom":{"keep":true}}"#,
    );
    let output = f.ok(&["init", "vite", "--json"]);
    assert!(output.contains("\"name\": \"keep\""));
    assert!(output.contains("\"version\": \"2.0.0\""));
    assert!(output.contains("\"keep\": true"));
    f.ok(&["init", "nested/app", "--prefix", "base"]);
    assert!(f.root.join("base/nested/app/package.json").is_file());
    let absolute = f.root.join("absolute project");
    f.ok(&["init", absolute.to_str().unwrap(), "--json"]);
    assert!(absolute.join("package.json").is_file());
    f.ok(&["init", "--", "--project"]);
    assert!(f.root.join("--project/package.json").is_file());
    for args in [
        &["create"][..],
        &["create", "-y"][..],
        &["init", "one", "two"][..],
    ] {
        let output = f.invoke(args);
        assert!(!output.status.success());
    }
    assert!(!f.root.join("one").exists());
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        parent
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn help_groups_aliases_with_their_canonical_commands() {
    let f = Fixture::new();
    for args in [&[][..], &["--help"][..]] {
        let help = f.ok(args);
        for (command, aliases) in [
            ("install", "(alias: i, add)"),
            ("uninstall", "(alias: remove, rm)"),
            ("exec", "(alias: x)"),
        ] {
            let line = help
                .lines()
                .find(|line| line.split_whitespace().next() == Some(command))
                .unwrap();
            assert!(line.contains(aliases), "{line}");
        }
        for alias in ["i", "add", "remove", "rm", "x"] {
            assert!(
                !help
                    .lines()
                    .any(|line| line.split_whitespace().next() == Some(alias)),
                "{help}"
            );
        }
        assert!(
            help.lines()
                .any(|line| line.split_whitespace().next() == Some("create"))
        );
    }
    for alias in ["i", "add", "remove", "rm", "x", "create"] {
        assert!(f.ok(&[alias, "--help"]).contains("Usage:"));
    }
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn create_runs_initializers_and_forwards_arguments() {
    let f = Fixture::new();
    f.single("create-starter", r#", "bin":"bin/cli.sh""#);
    f.single("@scope/create-starter", r#", "bin":"bin/cli.sh""#);
    let manifest = r#"{"name":"fixture","version":"1.0.0","custom":true}"#;
    f.file("package.json", manifest);

    {
        let command = "create";
        assert_eq!(
            f.ok(&[
                command,
                "starter@latest",
                "my-app",
                "--template",
                "react",
                "--json",
                "--help",
                "--version",
                "space value",
                "$(echo UNSAFE)",
            ]),
            "my-app\n--template\nreact\n--json\n--help\n--version\nspace value\n$(echo UNSAFE)\n"
        );
        assert_eq!(
            f.ok(&[
                command,
                "@scope/starter@1.0.0",
                "--",
                "my-app",
                "--template",
                "react",
            ]),
            "my-app\n--template\nreact\n"
        );
    }
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        manifest
    );
    assert!(!f.root.join("node_modules").exists());
    assert!(!f.root.join("package-lock.json").exists());
    let requests = f.requests.lock().unwrap();
    assert!(requests.iter().any(|path| path == "/create-starter"));
    assert!(
        requests
            .iter()
            .any(|path| path == "/@scope%2Fcreate-starter")
    );
}

#[test]
fn version_updates_manifest_and_lockfiles_without_losing_custom_fields() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"name":"fixture","version":"1.2.3","custom":{"keep":true}}"#,
    );
    for name in ["package-lock.json", "npm-shrinkwrap.json", "tnpm-lock.json"] {
        f.file(name, r#"{"name":"fixture","version":"1.2.3","lockfileVersion":3,"packages":{"":{"version":"1.2.3"},"node_modules/app":{"version":"9.0.0"}}}"#);
    }
    f.file("npm-shrinkwrap.json", "not valid json");

    assert_eq!(
        f.ok(&["version", "patch", "--no-git-tag-version", "--json"]),
        "\"1.2.4\"\n"
    );
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains("\"version\": \"1.2.4\""));
    assert!(manifest.contains("\"keep\": true"));
    let lock = fs::read_to_string(f.root.join("tnpm-lock.json")).unwrap();
    assert_eq!(lock.matches("\"version\": \"1.2.4\"").count(), 2);
    let lock = fs::read_to_string(f.root.join("package-lock.json")).unwrap();
    assert!(lock.contains("1.2.3"));
    assert!(!lock.contains("1.2.4"));
    assert_eq!(
        fs::read_to_string(f.root.join("npm-shrinkwrap.json")).unwrap(),
        "not valid json"
    );
    assert!(f.ok(&["version", "--json"]).contains("1.2.4"));
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn add_save_flags_omit_dev_and_exec_remote_preserve_manifest() {
    let f = Fixture::new();
    f.file("package.json", r#"{"name":"fixture","version":"1.0.0"}"#);
    f.single("app", r#", "bin":"bin/cli.sh""#);
    f.ok(&["add", "app", "-D", "-E", "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains("devDependencies"));
    assert!(manifest.contains("\"app\": \"1.0.0\""));
    f.ok(&["ci", "--omit=dev", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/app").exists());
    let output = f.ok(&["exec", "--yes", "app", "remote argument"]);
    assert!(output.ends_with("remote argument\n"));
    assert_eq!(
        fs::read_to_string(f.root.join("package.json")).unwrap(),
        manifest
    );
}

#[test]
fn run_preserves_exit_code_and_ignore_scripts_runs_requested_script() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"scripts":{"fail":"exit 7","preok":"exit 9","ok":"echo ok"}}"#,
    );
    let failed = f.invoke(&["run", "fail"]);
    assert_eq!(failed.status.code(), Some(7));
    let stderr = String::from_utf8(failed.stderr).unwrap();
    assert!(stderr.contains("tnpm: fail script failed"), "{stderr}");
    assert!(
        stderr.contains("  caused by: command exited with"),
        "{stderr}"
    );
    assert_eq!(f.invoke(&["run", "ok"]).status.code(), Some(9));
    assert_eq!(f.ok(&["run", "--ignore-scripts", "ok"]), "ok\n");
    f.ok(&["run", "--if-present", "missing"]);
}
