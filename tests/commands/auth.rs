use crate::support::Fixture;
use std::{
    fs,
    io::Write,
    process::{Output, Stdio},
};

fn credentials(f: &Fixture, path: &str, token: &str) {
    f.file(
        "user.npmrc",
        &format!(
            "//{}{path}:_authToken={token}\n",
            f.url.trim_start_matches("http://")
        ),
    );
}

fn login(f: &Fixture, command: &str, input: &str, extra: &[&str]) -> Output {
    let mut child = f
        .command()
        .args([
            command,
            "--auth-type=legacy",
            "--userconfig=user.npmrc",
            "--json",
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn legacy_login_atomically_saves_only_user_token_and_preserves_other_settings() {
    let f = Fixture::new();
    let path = "/team/-/user/org.couchdb.user:alice";
    f.routes
        .lock()
        .unwrap()
        .insert(path.into(), br#"{"token":"secret#with;comments"}"#.to_vec());
    f.file("user.npmrc", &format!("# preserve this\ncolor=false\n//{}team/:username=old\n//{}team/:_password=b2xk\n//elsewhere.test/:_authToken=other\n", f.url.trim_start_matches("http://"), f.url.trim_start_matches("http://")));
    f.file(".npmrc", "tag=beta\n");
    let registry = format!("{}team/", f.url);
    let out = login(
        &f,
        "login",
        "alice\nsecret-password\n",
        &["--registry", &registry, "--scope=@team", "--otp=123456"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "{\"ok\":true}");
    let text = fs::read_to_string(f.root.join("user.npmrc")).unwrap();
    assert!(text.contains("# preserve this\ncolor=false\n"));
    assert!(text.contains("//elsewhere.test/:_authToken=other"));
    assert!(text.contains("@team:registry="));
    assert!(
        !text.contains("secret-password")
            && !text.contains("username=old")
            && !text.contains("_password=")
    );
    assert_eq!(
        fs::read_to_string(f.root.join(".npmrc")).unwrap(),
        "tag=beta\n"
    );
    let writes = f.writes.lock().unwrap();
    assert_eq!((&*writes[0].0, &*writes[0].1), ("PUT", path));
    assert!(String::from_utf8_lossy(&writes[0].2).contains("secret-password"));
    assert!(writes[0].3.to_ascii_lowercase().contains("npm-otp: 123456"));
    drop(writes);
    f.routes
        .lock()
        .unwrap()
        .insert("/team/-/whoami".into(), br#"{"username":"alice"}"#.to_vec());
    assert_eq!(
        f.ok(&[
            "whoami",
            "--scope=@team",
            "--userconfig=user.npmrc",
            "--json"
        ]),
        "\"alice\"\n"
    );
    assert!(
        f.authorizations
            .lock()
            .unwrap()
            .iter()
            .any(|(path, auth)| path == "/team/-/whoami"
                && auth.as_deref() == Some("Bearer secret#with;comments"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(f.root.join("user.npmrc"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!combined.contains("secret-password") && !combined.contains("secret#with;comments"));
}

#[test]
fn adduser_sends_email_and_failed_login_does_not_modify_credentials() {
    let f = Fixture::new();
    f.routes.lock().unwrap().insert(
        "/-/user/org.couchdb.user:alice".into(),
        br#"{"token":"new-token"}"#.to_vec(),
    );
    let out = login(&f, "adduser", "alice\npassword\nalice@example.test\n", &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&f.writes.lock().unwrap()[0].2).contains("alice@example.test"));
    let old = fs::read(f.root.join("user.npmrc")).unwrap();
    f.statuses
        .lock()
        .unwrap()
        .insert("/-/user/org.couchdb.user:alice".into(), 401);
    let out = login(&f, "login", "alice\npassword\n", &[]);
    assert!(!out.status.success());
    assert_eq!(fs::read(f.root.join("user.npmrc")).unwrap(), old);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("new-token"));
}

#[test]
fn legacy_login_retries_couch_revision_conflict_with_basic_auth() {
    let f = Fixture::new();
    let path = "/-/user/org.couchdb.user:alice";
    f.statuses.lock().unwrap().insert(path.into(), 409);
    f.routes.lock().unwrap().insert(
        format!("{path}?write=true"),
        br#"{"_rev":"1-test","roles":[]}"#.to_vec(),
    );
    f.routes.lock().unwrap().insert(
        format!("{path}/-rev/1-test"),
        br#"{"token":"new-token"}"#.to_vec(),
    );
    let out = login(&f, "login", "alice\npassword\n", &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        f.authorizations
            .lock()
            .unwrap()
            .iter()
            .any(|(path, auth)| path.ends_with("/-rev/1-test")
                && auth.as_deref() == Some("Basic YWxpY2U6cGFzc3dvcmQ="))
    );
}

#[test]
fn web_login_prints_browser_url_and_polls_without_existing_token() {
    let f = Fixture::new();
    credentials(&f, "", "old-token");
    f.routes.lock().unwrap().insert(
        "/-/v1/login".into(),
        format!(
            r#"{{"loginUrl":"{}browser","doneUrl":"{}done?session=1"}}"#,
            f.url, f.url
        )
        .into_bytes(),
    );
    f.routes.lock().unwrap().insert(
        "/done?session=1".into(),
        br#"{"token":"web-token"}"#.to_vec(),
    );
    let out = f.invoke(&["login", "--userconfig=user.npmrc", "--json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("browser"));
    assert!(
        fs::read_to_string(f.root.join("user.npmrc"))
            .unwrap()
            .contains("web-token")
    );
    assert!(
        f.authorizations
            .lock()
            .unwrap()
            .iter()
            .all(|(_, auth)| auth.is_none())
    );
}

#[test]
fn web_login_falls_back_to_legacy_only_for_unsupported_endpoint() {
    let f = Fixture::new();
    f.routes.lock().unwrap().insert(
        "/-/user/org.couchdb.user:alice".into(),
        br#"{"token":"fallback-token"}"#.to_vec(),
    );
    let out = login(&f, "login", "alice\npassword\n", &["--auth-type=web"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("using legacy"));
    assert!(
        fs::read_to_string(f.root.join("user.npmrc"))
            .unwrap()
            .contains("fallback-token")
    );
    f.statuses.lock().unwrap().insert("/-/v1/login".into(), 401);
    let before = f.writes.lock().unwrap().len();
    let out = login(&f, "login", "alice\npassword\n", &["--auth-type=web"]);
    assert!(!out.status.success());
    assert_eq!(f.writes.lock().unwrap().len(), before + 1);
}

#[test]
fn web_login_retries_accepted_responses_until_the_session_is_completed() {
    let f = Fixture::new();
    f.routes.lock().unwrap().insert(
        "/-/v1/login".into(),
        format!(
            r#"{{"loginUrl":"{}browser","doneUrl":"{}done"}}"#,
            f.url, f.url
        )
        .into_bytes(),
    );
    f.routes
        .lock()
        .unwrap()
        .insert("/done".into(), br#"{"token":"polled-token"}"#.to_vec());
    f.statuses.lock().unwrap().insert("/done".into(), 202);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while f
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|p| *p == "/done")
                .count()
                < 2
            {
                assert!(std::time::Instant::now() < deadline, "poll was not started");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            f.statuses.lock().unwrap().remove("/done");
        });
        f.ok(&["login", "--userconfig=user.npmrc"]);
    });
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == "/done")
            .count()
            >= 2
    );
}

#[test]
fn unsafe_polling_and_login_redirects_never_receive_credentials() {
    let f = Fixture::new();
    let other = Fixture::new();
    f.file("user.npmrc", "# original\n");
    f.routes.lock().unwrap().insert(
        "/-/v1/login".into(),
        format!(
            r#"{{"loginUrl":"{}browser","doneUrl":"{}done"}}"#,
            f.url, other.url
        )
        .into_bytes(),
    );
    assert!(
        !f.invoke(&["login", "--userconfig=user.npmrc"])
            .status
            .success()
    );
    f.redirects.lock().unwrap().insert(
        "/-/user/org.couchdb.user:alice".into(),
        format!("{}stolen", other.url),
    );
    let out = login(&f, "login", "alice\npassword\n", &[]);
    assert!(!out.status.success());
    assert!(other.requests.lock().unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(f.root.join("user.npmrc")).unwrap(),
        "# original\n"
    );
}

#[test]
fn logout_revokes_selected_token_then_removes_only_its_user_entries() {
    let f = Fixture::new();
    credentials(&f, "team/", "selected-token");
    let original = fs::read(f.root.join("user.npmrc")).unwrap();
    let registry = format!("{}team/", f.url);
    let args = ["logout", "--registry", &registry, "--userconfig=user.npmrc"];
    let out = f.invoke(&args);
    assert!(!out.status.success());
    assert_eq!(fs::read(f.root.join("user.npmrc")).unwrap(), original);
    assert!(!String::from_utf8_lossy(&out.stderr).contains("selected-token"));
    f.routes
        .lock()
        .unwrap()
        .insert("/team/-/user/token/selected-token".into(), b"{}".to_vec());
    f.ok(&args);
    assert!(
        !fs::read_to_string(f.root.join("user.npmrc"))
            .unwrap()
            .contains("selected-token")
    );
    assert!(
        f.writes
            .lock()
            .unwrap()
            .iter()
            .any(|(method, path, _, _)| method == "DELETE"
                && path == "/team/-/user/token/selected-token")
    );
}

#[test]
fn whoami_supports_basic_auth_and_rejects_missing_auth_or_offline() {
    let f = Fixture::new();
    for args in [
        vec!["whoami"],
        vec!["login", "--offline"],
        vec!["whoami", "extra"],
    ] {
        assert!(!f.invoke(&args).status.success());
    }
    f.file(
        ".npmrc",
        &format!(
            "//{}:_auth=YWxpY2U6cGFzc3dvcmQ=\n",
            f.url.trim_start_matches("http://")
        ),
    );
    assert_eq!(f.ok(&["whoami"]), "alice\n");
    assert!(f.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn credential_writes_refuse_symlink_userconfig() {
    let f = Fixture::new();
    f.file("real.npmrc", "# unchanged\n");
    std::os::unix::fs::symlink(f.root.join("real.npmrc"), f.root.join("user.npmrc")).unwrap();
    f.routes.lock().unwrap().insert(
        "/-/user/org.couchdb.user:alice".into(),
        br#"{"token":"new-token"}"#.to_vec(),
    );
    assert!(
        !login(&f, "login", "alice\npassword\n", &[])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(f.root.join("real.npmrc")).unwrap(),
        "# unchanged\n"
    );
}

#[test]
fn logout_removes_array_credentials_without_reactivating_an_old_token() {
    let f = Fixture::new();
    let host = f.url.trim_start_matches("http://");
    f.file(
        "user.npmrc",
        &format!("//{host}:_authToken[]=old-token\n//{host}:_authToken=current-token\n"),
    );
    f.routes
        .lock()
        .unwrap()
        .insert("/-/user/token/current-token".into(), b"{}".to_vec());
    f.ok(&["logout", "--userconfig=user.npmrc"]);
    assert!(
        !fs::read_to_string(f.root.join("user.npmrc"))
            .unwrap()
            .contains("Token")
    );
    assert!(
        !f.invoke(&["whoami", "--userconfig=user.npmrc"])
            .status
            .success()
    );
}
