//! CLI integration tests: config.
use crate::support::Fixture;

#[test]
fn npmrc_layers_arrays_comments_and_environment_expansion() {
    let f = Fixture::new();
    f.file("global.npmrc", "tag=global\nomit[]=optional\nomit[]=peer\n");
    f.file("user.npmrc", "tag=user\nomit[]=dev\n");
    f.file(".npmrc", "# comment\n; comment\ntag = project ; inline comment\nnode-options = \"${TNPM_TEST_UNDEFINED?} --use-system-ca\"\ncache = ${HOME}/custom-cache\nunknown = ${TNPM_TEST_UNDEFINED}\nquoted = 'a#b;c'\nescaped = a\\#b\\;c\n");
    let get = |args: &[&str]| {
        let output = f
            .command()
            .args(args)
            .args([
                "--userconfig",
                "user.npmrc",
                "--globalconfig",
                "global.npmrc",
            ])
            .env_remove("TNPM_TEST_UNDEFINED")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    assert_eq!(get(&["get", "tag"]), "project\n");
    assert_eq!(get(&["get", "omit", "--json"]), "[\n  \"dev\"\n]\n");
    assert_eq!(get(&["get", "node-options"]), " --use-system-ca\n");
    assert_eq!(get(&["get", "unknown"]), "${TNPM_TEST_UNDEFINED}\n");
    assert_eq!(get(&["get", "quoted", "escaped"]), "a#b;c\na#b;c\n");
    assert_eq!(
        get(&["get", "cache"]),
        format!("{}/home/custom-cache\n", f.root.display())
    );
    let output = f
        .command()
        .args([
            "get",
            "tag",
            "--userconfig",
            "user.npmrc",
            "--globalconfig",
            "global.npmrc",
        ])
        .env("NPM_CONFIG_TAG", "environment")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "environment\n");
    assert_eq!(
        get(&["get", "registry", "--registry", "https://cli.test/"]),
        "https://cli.test/\n"
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn npmrc_user_prefix_selects_global_file_and_arrays_drive_omit() {
    let f = Fixture::new();
    f.file("home/.npmrc", "prefix=${HOME}/custom-prefix\n");
    f.file("home/custom-prefix/etc/npmrc", "tag=from-global\n");
    let output = f
        .command()
        .args(["get", "tag"])
        .env_remove("npm_config_userconfig")
        .env_remove("npm_config_globalconfig")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "from-global\n");

    f.single("prod", "");
    f.single("dev", "");
    f.single("optional", "");
    f.file("package.json", r#"{"dependencies":{"prod":"1"},"devDependencies":{"dev":"1"},"optionalDependencies":{"optional":"1"}}"#);
    f.file(".npmrc", "omit[]=dev\nomit[]=optional\n");
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/prod").is_dir());
    assert!(!f.root.join("node_modules/dev").exists());
    assert!(!f.root.join("node_modules/optional").exists());
}

#[test]
fn scoped_auth_is_shared_by_view_and_install_and_secrets_are_protected() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    let host = f.url.strip_prefix("http://").unwrap();
    for auth in [
        "_authToken=fixture-token",
        "_auth=dXNlcjpwYXNz",
        "username=user\nPLACEHOLDER_password=cGFzcw==",
    ] {
        let auth = auth.replace("PLACEHOLDER", &format!("//{host}:"));
        f.file(".npmrc", &format!("//{host}:{auth}\n"));
        f.ok(&["view", "app", "version", "--json"]);
        f.ok(&["install", "--ignore-scripts"]);
        let output = f.ok(&["get", "--json"]);
        for secret in ["fixture-token", "dXNlcjpwYXNz", "cGFzcw==", "username"] {
            assert!(!output.contains(secret));
        }
    }
    let auths = f.authorizations.lock().unwrap();
    assert!(
        auths
            .iter()
            .any(|(_, auth)| auth.as_deref() == Some("Bearer fixture-token"))
    );
    assert!(
        auths
            .iter()
            .any(|(_, auth)| auth.as_deref() == Some("Basic dXNlcjpwYXNz"))
    );
    drop(auths);
    f.file(".npmrc", "_authToken=do-not-leak\n");
    let output = f.invoke(&["view", "app"]);
    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("must be scoped"));
    assert!(!error.contains("do-not-leak"));
}

#[test]
fn authenticated_redirects_cannot_escape_registry_path_scope() {
    let f = Fixture::new();
    let host = f.url.strip_prefix("http://").unwrap();
    f.file(
        ".npmrc",
        &format!(
            "registry={}private/\n//{host}private/:_authToken=fixture-token\n",
            f.url
        ),
    );
    f.redirects
        .lock()
        .unwrap()
        .insert("/private/app".into(), format!("{}outside/app", f.url));
    let output = f
        .command()
        .args(["view", "app"])
        .env_remove("npm_config_registry")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(*f.requests.lock().unwrap(), ["/private/app"]);
}

#[test]
fn missing_auth_environment_variables_fail_without_sending_requests_or_secrets() {
    let f = Fixture::new();
    let host = f.url.strip_prefix("http://").unwrap();
    f.file(
        ".npmrc",
        &format!("//{host}:_authToken=${{TNPM_TEST_MISSING_TOKEN}}\n"),
    );
    let output = f
        .command()
        .args(["view", "app"])
        .env_remove("TNPM_TEST_MISSING_TOKEN")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("authentication environment variable is not set")
    );
    assert!(f.requests.lock().unwrap().is_empty());
}
