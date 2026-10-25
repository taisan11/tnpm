//! CLI integration tests: outdated.
use crate::support;
use crate::support::Fixture;

#[test]
fn outdated_borders_plain_and_colors_do_not_change_json() {
    use unicode_width::UnicodeWidthStr;
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    let run = |args: &[&str], no_color| {
        let mut command = f.command();
        command.args(args).env("CLICOLOR_FORCE", "1");
        if no_color {
            command.env("NO_COLOR", "1");
        }
        command.output().unwrap()
    };
    let output = run(&["outdated"], false);
    assert_eq!(output.status.code(), Some(1));
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.starts_with('┌'));
    assert!(table.contains('\x1b'));
    let clean = support::strip_ansi(&table);
    let width = clean.lines().next().unwrap().width();
    assert!(clean.lines().all(|line| line.width() == width));
    let output = run(&["outdated", "--plain"], false);
    let plain = String::from_utf8(output.stdout).unwrap();
    assert!(plain.starts_with("Package"));
    assert!(!plain.contains('│') && !plain.contains('\x1b'));
    let output = run(&["outdated", "--json"], false);
    let json = String::from_utf8(output.stdout).unwrap();
    nojson::RawJson::parse(&json).unwrap();
    assert!(!json.contains('\x1b'));
    let output = run(&["outdated"], true);
    assert!(!output.stdout.contains(&0x1b));
    assert!(!output.stderr.contains(&0x1b));
}

#[test]
fn outdated_hides_latest_installed_versions_even_when_wanted_is_older() {
    let f = Fixture::new();
    f.releases("pinned", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    for name in ["up-to-date", "stale", "missing"] {
        f.single(name, "");
    }
    f.file(
        "package.json",
        r#"{"dependencies":{"up-to-date":"1","pinned":"^1","stale":"1","missing":"1"}}"#,
    );
    f.file(
        "node_modules/up-to-date/package.json",
        r#"{"name":"up-to-date","version":"1.0.0"}"#,
    );
    f.file(
        "node_modules/pinned/package.json",
        r#"{"name":"pinned","version":"2.0.0"}"#,
    );
    f.file(
        "node_modules/stale/package.json",
        r#"{"name":"stale","version":"0.9.0"}"#,
    );

    for args in [&["outdated"][..], &["outdated", "--json"][..]] {
        let output = f.invoke(args);
        assert_eq!(output.status.code(), Some(1));
        let output = String::from_utf8(output.stdout).unwrap();
        assert!(!output.contains("up-to-date"), "{output}");
        assert!(!output.contains("pinned"), "{output}");
        assert!(output.contains("stale"), "{output}");
        assert!(output.contains("missing"), "{output}");
    }
    assert_eq!(
        f.ok(&["outdated", "up-to-date", "pinned"]),
        "All dependencies are up to date.\n"
    );
    assert_eq!(
        f.ok(&["outdated", "up-to-date", "pinned", "--json"]),
        "{}\n"
    );
}

#[test]
fn outdated_does_not_print_partial_results_on_failure() {
    let f = Fixture::new();
    f.single("short", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"short":"1","unknown":"1"}}"#,
    );
    let output = f.invoke(&["outdated"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
