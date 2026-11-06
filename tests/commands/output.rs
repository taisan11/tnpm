//! CLI integration tests: output.
#[cfg(unix)]
use crate::support;
use crate::support::Fixture;

#[test]
#[cfg(unix)]
fn run_supports_separator_before_script_and_exposes_original_lifecycle_script() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"scripts":{"pretest":"printf 'pre:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\"","test":"printf 'main:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\"","posttest":"printf 'post:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\""}}"#,
    );
    let expected = concat!(
        "pre:pretest:printf 'pre:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\"\n",
        "main:test:printf 'main:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\"\n",
        "post:posttest:printf 'post:%s:%s\\n' \"$npm_lifecycle_event\" \"$npm_lifecycle_script\"\n",
    );
    assert_eq!(f.ok(&["run", "--", "test"]), expected);
    assert!(!f.invoke(&["run", "--", "missing"]).status.success());
    assert!(
        f.invoke(&["run", "--if-present", "--", "missing"])
            .status
            .success()
    );
}

#[test]
#[cfg(unix)]
fn run_separator_forwards_arguments_without_interpreting_flags() {
    let f = Fixture::new();
    f.file("package.json", r#"{"scripts":{"test":"printf '<%s>\\n'"}}"#);
    for args in [
        &["run", "--", "test", "hello world", "--json"][..],
        &["run", "test", "--", "hello world", "--json"][..],
    ] {
        assert_eq!(f.ok(args), "<hello world>\n<--json>\n");
    }
    f.file(
        "package.json",
        r#"{"scripts":{"test":"printf '%s\\n' \"$npm_lifecycle_script\"; :"}}"#,
    );
    assert_eq!(
        f.ok(&["run", "--", "test", "extra argument"]),
        "printf '%s\\n' \"$npm_lifecycle_script\"; :\n"
    );
}

#[test]
#[cfg(unix)]
fn quiet_and_printing_lifecycle_scripts_do_not_add_blank_terminal_lines() {
    let f = Fixture::new();
    f.single("app", r#", "scripts":{"postinstall":":"}"#);
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"preinstall":":","install":":","postinstall":":"}}"#);
    let output = f.invoke_tty(&["install", "--json"], "xterm-256color");
    assert!(output.status.success());
    nojson::RawJson::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    let error =
        support::strip_ansi(&String::from_utf8(output.stderr).unwrap()).replace("\r\n", "\n");
    assert_eq!(
        error.matches('\n').count(),
        2,
        "quiet hooks must reuse the progress row: {error:?}"
    );
    assert!(!error.contains("\n\n"), "{error:?}");
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"preinstall":"echo complete-line-marker","install":":","postinstall":"printf partial-line-marker"}}"#);
    let output = f.invoke_tty(&["update", "--json"], "xterm-256color");
    assert!(output.status.success());
    nojson::RawJson::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    let error =
        support::strip_ansi(&String::from_utf8(output.stderr).unwrap()).replace("\r\n", "\n");
    assert!(
        error.lines().any(|line| line == "complete-line-marker"),
        "{error:?}"
    );
    assert!(
        error.lines().any(|line| line == "partial-line-marker"),
        "{error:?}"
    );
    assert!(!error.contains("\n\n"), "{error:?}");
}

#[test]
fn every_command_finishes_with_build_version_on_stderr_and_preserves_exit_codes() {
    let f = Fixture::new();
    f.file("package.json", r#"{"scripts":{"fail":"exit 7"}}"#);
    let footer = format!(
        "tnpm v{}({})",
        env!("CARGO_PKG_VERSION"),
        env!("TNPM_BUILD_COMMIT")
    );
    for (args, code) in [
        (&["--help"][..], 0),
        (&["cache", "--json"][..], 0),
        (&["install", "--ignore-scripts", "--json"][..], 0),
        (&["unknown-command"][..], 1),
        (&["run", "fail"][..], 7),
        (&["view", "unknown"][..], 1),
    ] {
        let output = f
            .command()
            .args(args)
            .env("TNPM_BUILD_COMMIT", "runtime-value-must-not-be-used")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(code), "{args:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            error.lines().last(),
            Some(footer.as_str()),
            "{args:?}: {error}"
        );
        assert_eq!(error.lines().filter(|line| *line == footer).count(), 1);
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.contains(&footer));
        if args.contains(&"--json") {
            nojson::RawJson::parse(&stdout).unwrap();
        }
    }
}

#[test]
fn install_logs_network_cache_extraction_and_placement_progress_on_stderr() {
    use std::collections::BTreeMap;
    let f = Fixture::new();
    f.single("app", r#", "dependencies":{"child":"1"}"#);
    f.single("child", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    let output = f.invoke(&["install", "--ignore-scripts", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    nojson::RawJson::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!error.contains('\r'));
    assert!(!error.contains('\x1b'));
    for expected in [
        "HTTP GET metadata app",
        "HTTP GET metadata child",
        "HTTP GET tarball app",
        "HTTP GET tarball child",
        "extract node_modules/app",
        "place:",
        "node_modules/child",
    ] {
        assert!(error.contains(expected), "{error}");
    }
    assert!(!error.contains("http://"), "{error}");
    assert!(!error.contains("https://"), "{error}");
    let mut phases = BTreeMap::new();
    for line in error.lines() {
        let Some(rest) = line.strip_prefix("tnpm: [") else {
            continue;
        };
        let (counts, detail) = rest.split_once("] ").unwrap();
        let (done, total) = counts.split_once('/').unwrap();
        let (done, total) = (
            done.parse::<usize>().unwrap(),
            total.parse::<usize>().unwrap(),
        );
        assert!(done <= total);
        let (phase, _) = detail.split_once(": ").unwrap();
        let previous = phases.insert(phase, (done, total)).unwrap_or_default();
        assert!(previous.0 <= done && previous.1 <= total, "{line}");
    }
    assert_eq!(phases["resolve"], (2, 2));
    assert_eq!(phases["prepare"], (2, 2));
    assert_eq!(phases["place"], (2, 2));
    let output = f.invoke(&["install", "--ignore-scripts"]);
    assert!(String::from_utf8_lossy(&output.stderr).contains("no packages need placement"));
    let output = f.invoke(&["ci", "--ignore-scripts"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("clone package cache"));
}

#[test]
#[cfg(unix)]
fn terminal_progress_replaces_one_row_without_hiding_warnings_scripts_or_footer() {
    let f = Fixture::new();
    f.file(".npmrc", "legacy-peer-deps=false\n");
    f.single("plugin", r#", "peerDependencies":{"host":"^1"}"#);
    let host = f.package("host", "2.0.0", "");
    f.registry("host", "2.0.0", &[("2.0.0", host)]);
    f.file("package.json", r#"{"dependencies":{"plugin":"1","host":"2"},"scripts":{"postinstall":"printf terminal-script-marker"}}"#);
    let output = f.invoke_tty(&["install", "--json"], "xterm-256color");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success(), "{error}");
    nojson::RawJson::parse(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert!(error.contains("\r\x1b[2K"), "{error:?}");
    let error = support::strip_ansi(&error);
    let warning = error
        .find("tnpm: warning: overriding peer dependency")
        .unwrap();
    assert!(error[..warning].ends_with('\n'), "{error:?}");
    assert!(
        error.lines().any(|line| line == "terminal-script-marker"),
        "{error:?}"
    );
    assert_eq!(
        error.lines().last(),
        Some(
            format!(
                "tnpm v{}({})",
                env!("CARGO_PKG_VERSION"),
                env!("TNPM_BUILD_COMMIT")
            )
            .as_str()
        )
    );
}

#[test]
#[cfg(unix)]
fn dumb_terminals_keep_plain_progress_logs() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    let output = f.invoke_tty(&["install", "--ignore-scripts"], "dumb");
    assert!(output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(!error.contains('\x1b'));
    assert!(error.contains("tnpm: [0/0] resolve:"));
}

#[test]
fn install_and_update_json_keep_lifecycle_output_on_stderr() {
    let f = Fixture::new();
    f.single(
        "app",
        r#", "scripts":{"install":"echo dependency-install-marker"}"#,
    );
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"preinstall":"echo root-preinstall-marker","postinstall":"echo root-postinstall-marker","echo":"echo explicit-run-marker"}}"#);
    for command in ["install", "update"] {
        let output = f.invoke(&[command, "--json"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        nojson::RawJson::parse(&stdout).unwrap();
        assert!(!stdout.contains("marker"));
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("scripts:"));
        assert!(error.contains("root-preinstall-marker"));
        assert!(error.contains("root-postinstall-marker"));
        if command == "install" {
            assert!(error.contains("dependency-install-marker"));
        }
    }
    let output = f.invoke(&["update", "--json", "--ignore-scripts"]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("marker"));
    assert_eq!(f.ok(&["run", "echo"]), "explicit-run-marker\n");
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"preinstall":"echo failed-script-marker; exit 9"}}"#);
    let output = f.invoke(&["update", "--json"]);
    assert_eq!(output.status.code(), Some(9));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed-script-marker"));
}

#[test]
fn malformed_json_reports_the_file_and_original_cause() {
    let f = Fixture::new();
    for file in ["package.json", "tnpm-lock.json"] {
        f.file("package.json", r#"{"name":"fixture"}"#);
        f.file(file, "[");
        let out = f.invoke(&["install", "--ignore-scripts"]);
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.starts_with("tnpm: could not parse "), "{stderr}");
        assert!(stderr.contains(file), "{stderr}");
        assert!(stderr.contains("  caused by: "), "{stderr}");
        assert!(!f.root.join("node_modules").exists());
    }
}
