//! CLI integration tests: registry.
use crate::support::Fixture;

#[test]
fn registry_utf8_validation_rejects_invalid_input_without_poisoning_cache() {
    let f = Fixture::new();
    let mut bytes = br#"{"description":""#.to_vec();
    bytes.extend_from_slice(&[b'a'; 4096]);
    bytes.push(0xff);
    bytes.extend_from_slice(br#""}"#);
    f.routes.lock().unwrap().insert("/app".into(), bytes);
    let output = f.invoke(&["view", "app", "--json"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(
        error.contains("registry metadata is not valid UTF-8"),
        "{error}"
    );
    assert!(error.contains("caused by:"), "{error}");
    let requests = f.requests.lock().unwrap().len();
    f.single("app", "");
    assert_eq!(f.ok(&["view", "app", "version", "--json"]), "\"1.0.0\"\n");
    assert_eq!(f.requests.lock().unwrap().len(), requests + 1);
}

#[test]
fn registry_and_binary_cache_preserve_long_multibyte_utf8_metadata() {
    let f = Fixture::new();
    let description = "日本語👩‍💻e\u{301}".repeat(128);
    f.single("app", &format!(r#", "description":"{description}""#));
    let output = f.ok(&["view", "app", "description", "--json"]);
    assert_eq!(output, format!("\"{description}\"\n"));
    let requests = f.requests.lock().unwrap().len();
    assert_eq!(f.ok(&["view", "app", "description", "--json"]), output);
    assert_eq!(f.requests.lock().unwrap().len(), requests);
}

#[test]
fn projected_and_full_view_outputs_match_network_after_a_minimal_cache_warmup() {
    let f = Fixture::new();
    f.routes.lock().unwrap().insert("/app".into(), br#"{"name":"root-name","license":"root-license","time":{"created":"2026-10-25T00:00:00Z"},"maintainers":[{"email":"one@example.test"},{"email":"two@example.test"}],"readme":"top-level readme","dist-tags":{"latest":"2.0.0"},"versions":{"2.0.0":{"name":"app","version":"2.0.0","license":"selected-license","dist":{"tarball":"https://example.test/app.tgz"}},"1.0.0":{"name":"app","version":"1.0.0"}}}"#.to_vec());
    assert_eq!(f.ok(&["view", "app", "version", "--json"]), "\"2.0.0\"\n");
    for fields in [
        &[][..],
        &["license"][..],
        &["maintainers[1].email"][..],
        &["not-present"][..],
        &["time.created", "name", "version", "versions", "license"][..],
    ] {
        let mut args = vec!["view", "app"];
        args.extend_from_slice(fields);
        args.push("--json");
        let before = f.requests.lock().unwrap().len();
        let cached = f.ok(&args);
        assert_eq!(f.requests.lock().unwrap().len(), before);
        args.push("--no-cache");
        let network = f.ok(&args);
        assert_eq!(cached, network, "fields: {fields:?}");
    }
    assert_eq!(
        f.ok(&["view", "app", "license", "--json"]),
        "\"selected-license\"\n"
    );
    assert_eq!(
        f.ok(&["view", "app", "versions[0]", "--json"]),
        "\"1.0.0\"\n"
    );
}

#[test]
fn fresh_registry_cache_hits_do_not_initialize_http_clients() {
    let f = Fixture::new();
    f.single("app", "");
    f.ok(&["view", "app", "version", "--json"]);
    let count = f.requests.lock().unwrap().len();
    // A missing CA file makes client initialization fail, even for this local
    // fixture. It must not be read when a fresh cache can answer the query.
    let invoke = |extra: &[&str]| {
        f.command()
            .args(["view", "app", "version", "--json"])
            .args(extra)
            .env("npm_config_cafile", f.root.join("does-not-exist.pem"))
            .output()
            .unwrap()
    };
    let cached = invoke(&[]);
    assert!(
        cached.status.success(),
        "{}",
        String::from_utf8_lossy(&cached.stderr)
    );
    assert_eq!(cached.stdout, b"\"1.0.0\"\n");
    let network = invoke(&["--no-cache"]);
    assert!(!network.status.success());
    assert!(String::from_utf8_lossy(&network.stderr).contains("could not read cafile"));
    assert_eq!(f.requests.lock().unwrap().len(), count);
}

#[test]
fn view_uses_local_manifest_scoped_selectors_and_shared_registry_cache() {
    let f = Fixture::new();
    f.releases("@scope/app", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.file("package.json", r#"{"name":"@scope/app"}"#);

    assert_eq!(f.ok(&["view", ".", "version", "--json"]), "\"2.0.0\"\n");
    let requests = f.requests.lock().unwrap().len();
    assert_eq!(
        f.ok(&["view", "@scope/app@^1", "version", "--json"]),
        "\"1.0.0\"\n"
    );
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    assert_eq!(
        f.ok(&[
            "view",
            "@scope/app@1.0.0",
            "version",
            "--json",
            "--no-cache"
        ]),
        "\"1.0.0\"\n"
    );
    assert_eq!(f.requests.lock().unwrap().len(), requests + 1);
}
