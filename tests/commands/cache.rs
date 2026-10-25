//! CLI integration tests: cache.
use crate::support::Fixture;
use std::fs;

fn json_number(text: &str, key: &str) -> u64 {
    let json = nojson::RawJson::parse(text).unwrap();
    let key = format!("\"{key}\"");
    json.value()
        .to_object()
        .unwrap()
        .find(|(name, _)| name.as_raw_str() == key)
        .unwrap()
        .1
        .as_raw_str()
        .parse()
        .unwrap()
}

#[test]
fn cache_reports_usage_and_removes_all_tnpm_caches_without_touching_other_data() {
    let f = Fixture::new();
    for (path, content) in [
        ("home/.tnpm/cache/v3/ab/metadata.bin", "12345"),
        ("home/.tnpm/cache/tarballs-v1/a.tgz", "1234"),
        ("home/.tnpm/cache/packages-v1/a/package/index.js", "1234567"),
        ("home/.tnpm/cache/v2/legacy.bin", "12"),
        ("home/.tnpm/cache/.partial", "1"),
        ("home/.tnpm/exec/key/node_modules/app/index.js", "123"),
        ("home/.tnpm/settings", "keep settings"),
        ("home/.npm/keep", "keep npm cache"),
        ("node_modules/app/package.json", "keep project modules"),
        ("node_modules/.package-lock.json", "keep hidden lockfile"),
        ("package-lock.json", "keep project lockfile"),
    ] {
        f.file(path, content);
    }
    // Cache maintenance must not depend on project JSON, npmrc, or --prefix.
    f.file("package.json", "[");
    f.file(
        ".npmrc",
        "registry=${TNPM_CACHE_TEST_MISSING_CONFIG_VARIABLE}\n",
    );
    let output = f.ok(&["cache", "--json", "--prefix", "does-not-exist"]);
    assert_eq!(json_number(&output, "bytes"), 22);
    assert_eq!(json_number(&output, "files"), 6);
    assert_eq!(json_number(&output, "symlinks"), 0);
    let _ = json_number(&output, "diskBytes");
    for label in ["v3", "v2", "tarballs-v1", "packages-v1", ".partial", "exec"] {
        assert!(output.contains(label), "{output}");
    }
    let text = f.ok(&["cache"]);
    for label in ["Total", "Disk usage", "Logical size", "22 B", "packages-v1"] {
        assert!(text.contains(label), "{text}");
    }
    let output = f.ok(&["cache", "rm", "--json"]);
    nojson::RawJson::parse(&output).unwrap();
    assert!(
        output.contains("\"removed\"") && output.contains("/cache") && output.contains("/exec")
    );
    assert!(!f.root.join("home/.tnpm/cache").exists());
    assert!(!f.root.join("home/.tnpm/exec").exists());
    for (path, content) in [
        ("home/.tnpm/settings", "keep settings"),
        ("home/.npm/keep", "keep npm cache"),
        ("node_modules/app/package.json", "keep project modules"),
        ("node_modules/.package-lock.json", "keep hidden lockfile"),
        ("package-lock.json", "keep project lockfile"),
        ("package.json", "["),
    ] {
        assert_eq!(fs::read_to_string(f.root.join(path)).unwrap(), content);
    }
    assert_eq!(json_number(&f.ok(&["cache", "--json"]), "bytes"), 0);
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn cache_handles_missing_directories_without_creating_them() {
    let f = Fixture::new();
    let output = f.ok(&["cache", "--json"]);
    assert_eq!(json_number(&output, "bytes"), 0);
    assert_eq!(json_number(&output, "diskBytes"), 0);
    assert_eq!(json_number(&output, "files"), 0);
    assert_eq!(f.ok(&["cache", "rm"]), "No tnpm caches to remove.\n");
    assert!(f.ok(&["cache", "rm", "--json"]).contains("\"removed\": []"));
    assert!(!f.root.join("home/.tnpm").exists());
}

#[test]
fn cache_rejects_extra_arguments_before_deleting_anything() {
    let f = Fixture::new();
    f.file("home/.tnpm/cache/keep", "keep");
    for args in [
        &["cache", "unknown"][..],
        &["cache", "rm", "extra"][..],
        &["cache", "rm", ".."][..],
        &["cache", "rm", "--", "outside"][..],
        &["cache", "--", "rm"][..],
    ] {
        let output = f.invoke(args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            fs::read_to_string(f.root.join("home/.tnpm/cache/keep")).unwrap(),
            "keep"
        );
    }
}

#[cfg(unix)]
#[test]
fn cache_inspection_and_removal_never_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.file("outside/keep", "outside cache contents must remain");
    f.file("home/.tnpm/cache/v3/local", "1");
    let outside = f.root.join("outside");
    symlink(&outside, f.root.join("home/.tnpm/cache/v3/link")).unwrap();
    symlink(&outside, f.root.join("home/.tnpm/exec")).unwrap();
    let output = f.ok(&["cache", "--json"]);
    assert_eq!(json_number(&output, "bytes"), 1);
    assert_eq!(json_number(&output, "files"), 1);
    assert_eq!(json_number(&output, "symlinks"), 2);
    f.ok(&["cache", "rm"]);
    assert!(fs::symlink_metadata(f.root.join("home/.tnpm/exec")).is_err());
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "outside cache contents must remain"
    );
    // Test a symlink at the main cache root too, including a broken link.
    for target in [&outside, &f.root.join("does-not-exist")] {
        symlink(target, f.root.join("home/.tnpm/cache")).unwrap();
        assert_eq!(json_number(&f.ok(&["cache", "--json"]), "files"), 0);
        f.ok(&["cache", "rm"]);
        assert!(fs::symlink_metadata(f.root.join("home/.tnpm/cache")).is_err());
    }
    assert!(outside.join("keep").is_file());
}

#[cfg(unix)]
#[test]
fn cache_refuses_a_symlinked_tnpm_parent() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.file("outside/cache/keep", "keep");
    symlink(f.root.join("outside"), f.root.join("home/.tnpm")).unwrap();
    for args in [&["cache"][..], &["cache", "rm"][..]] {
        let output = f.invoke(args);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("symlinked tnpm directory"));
        assert_eq!(
            fs::read_to_string(f.root.join("outside/cache/keep")).unwrap(),
            "keep"
        );
    }
}
