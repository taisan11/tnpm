use crate::support::Fixture;
use std::fs;

const ENDPOINT: &str = "/-/npm/v1/security/advisories/bulk";

fn advisory(name: &str, severity: &str, range: &str) -> String {
    format!(
        r#"{{"{name}":[{{"name":"{name}","id":123,"url":"https://example.test/advisory/123","title":"test advisory","severity":"{severity}","vulnerable_versions":"{range}"}}]}}"#
    )
}

#[test]
fn audit_posts_locked_versions_and_reports_transitives_aliases_and_thresholds() {
    let f = Fixture::new();
    f.single("parent", r#", "dependencies":{"vulnerable":"1"}"#);
    f.single("vulnerable", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"parent":"1","alias":"npm:vulnerable@1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    let lock = fs::read(f.root.join("tnpm-lock.json")).unwrap();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    f.routes.lock().unwrap().insert(
        ENDPOINT.into(),
        advisory("vulnerable", "high", "<2").into_bytes(),
    );
    let out = f.invoke(&["audit", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    let json = String::from_utf8(out.stdout).unwrap();
    nojson::RawJson::parse(&json).unwrap();
    assert!(json.contains("node_modules/alias"));
    assert!(json.contains("node_modules/vulnerable"));
    assert!(json.contains("\"high\": 1"));
    f.ok(&["audit", "--audit-level=critical"]);
    f.ok(&["audit", "--audit-level=none", "--json"]);
    let posts = f.posts.lock().unwrap();
    let (path, body) = &posts[0];
    assert_eq!(path, ENDPOINT);
    let body = String::from_utf8_lossy(body);
    assert!(body.contains("vulnerable") && body.contains("1.0.0") && !body.contains("alias"));
    assert_eq!(fs::read(f.root.join("tnpm-lock.json")).unwrap(), lock);
}

#[test]
fn audit_omit_filters_payload_and_nonregistry_sources_are_not_submitted() {
    let f = Fixture::new();
    f.single("prod", "");
    f.single("dev", "");
    f.single("optional", "");
    f.file(
        "local/package.json",
        r#"{"name":"local","version":"1.0.0"}"#,
    );
    f.file("package.json", r#"{"dependencies":{"prod":"1","local":"file:local"},"devDependencies":{"dev":"1"},"optionalDependencies":{"optional":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.routes
        .lock()
        .unwrap()
        .insert(ENDPOINT.into(), b"{}".to_vec());
    let json = f.ok(&["audit", "--omit=dev,optional", "--json"]);
    assert!(json.contains("\"skipped\": 1"));
    let posts = f.posts.lock().unwrap();
    let body = String::from_utf8_lossy(&posts[0].1);
    assert!(body.contains("prod"));
    for name in ["dev", "optional", "local"] {
        assert!(!body.contains(name), "{body}");
    }
}

#[test]
fn audit_uses_scope_specific_endpoints_and_credentials_without_leaking_names() {
    let f = Fixture::new();
    let private = Fixture::new();
    f.single("public", "");
    private.single("@team/private", "");
    f.file(
        ".npmrc",
        &format!(
            "@team:registry={}\n//{}/:_authToken=private-token\n",
            private.url,
            private
                .url
                .trim_start_matches("http://")
                .trim_end_matches('/')
        ),
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"public":"1","@team/private":"1"}}"#,
    );
    f.ok(&["install", "--ignore-scripts"]);
    for fixture in [&f, &private] {
        fixture
            .routes
            .lock()
            .unwrap()
            .insert(ENDPOINT.into(), b"{}".to_vec());
    }
    f.ok(&["audit"]);
    let public = f.posts.lock().unwrap();
    let secret = private.posts.lock().unwrap();
    assert!(!String::from_utf8_lossy(&public[0].1).contains("@team/private"));
    assert!(String::from_utf8_lossy(&secret[0].1).contains("@team/private"));
    assert!(private.authorizations.lock().unwrap().iter().any(|(path, auth)| path == ENDPOINT && auth.as_deref() == Some("Bearer private-token")));
}

#[test]
fn audit_rejects_offline_bad_responses_and_unsupported_fixes() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    let before = f.requests.lock().unwrap().len();
    for args in [
        vec!["audit", "--offline"],
        vec!["audit", "--audit-level=bad"],
        vec!["audit", "fix"],
    ] {
        assert!(!f.invoke(&args).status.success());
    }
    assert_eq!(f.requests.lock().unwrap().len(), before);
    for body in ["[]", "not json", r#"{"app":{}}"#, r#"{"app":[{}]}"#] {
        f.routes
            .lock()
            .unwrap()
            .insert(ENDPOINT.into(), body.as_bytes().to_vec());
        let out = f.invoke(&["audit", "--json"]);
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }
    f.routes.lock().unwrap().remove(ENDPOINT);
    assert!(!f.invoke(&["audit"]).status.success());
    f.routes.lock().unwrap().insert(
        ENDPOINT.into(),
        advisory("app", "critical", ">=2").into_bytes(),
    );
    assert!(f.ok(&["audit"]).contains("0 vulnerabilities"));
}

#[test]
fn audit_rejects_cross_origin_redirects_even_without_authentication() {
    let f = Fixture::new();
    let other = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.redirects
        .lock()
        .unwrap()
        .insert(ENDPOINT.into(), format!("{}leak", other.url));
    assert!(!f.invoke(&["audit"]).status.success());
    assert!(other.requests.lock().unwrap().is_empty());
}

#[test]
fn audit_preserves_registry_base_paths_and_config_thresholds() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.file(".npmrc", "audit-level=critical\n");
    let endpoint = format!("/team{ENDPOINT}");
    f.routes
        .lock()
        .unwrap()
        .insert(endpoint.clone(), advisory("app", "high", "*").into_bytes());
    f.ok(&["audit", "--registry", &format!("{}team/", f.url)]);
    assert_eq!(f.posts.lock().unwrap()[0].0, endpoint);
    assert!(
        !f.invoke(&[
            "audit",
            "--registry",
            &format!("{}team/", f.url),
            "--audit-level=high"
        ])
        .status
        .success()
    );
}
