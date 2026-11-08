use crate::support::Fixture;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha512};
use std::{collections::BTreeMap, fs, io::Read};

fn archive(bytes: &[u8]) -> BTreeMap<String, (Vec<u8>, u32)> {
    tar::Archive::new(flate2::read::GzDecoder::new(bytes))
        .entries()
        .unwrap()
        .map(|entry| {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().to_str().unwrap().to_owned();
            let mode = entry.header().mode().unwrap();
            let mut contents = Vec::new();
            entry.read_to_end(&mut contents).unwrap();
            (path, (contents, mode))
        })
        .collect()
}

fn string(json: &nojson::RawJson<'_>, path: &[&str]) -> String {
    json.value()
        .to_path_member(path)
        .unwrap()
        .required()
        .unwrap()
        .try_into()
        .unwrap()
}

fn auth(f: &Fixture) {
    f.file(
        ".npmrc",
        &format!(
            "//{}:_authToken=publish-secret\n",
            f.url.trim_start_matches("http://")
        ),
    );
}

#[test]
fn pack_honors_allowlist_mandatory_files_nested_ignores_and_secret_exclusions() {
    let f = Fixture::new();
    f.file("package.json", r#"{"name":"@team/demo","version":"1.2.3","files":["dist/**"],"main":"index.js","bin":{"demo":"cli.js"}}"#);
    for path in [
        "dist/index.js",
        "dist/debug.map",
        "dist/keep.txt",
        "dist/.env.production",
        "dist/.npmrc",
        "dist/key.pem",
        "dist/existing.tgz",
        "index.js",
        "cli.js",
        "README.md",
        "LICENSE",
        "source.js",
        ".env",
        ".npmrc",
        "node_modules/dep/index.js",
        "tnpm-lock.json",
        ".git/config",
    ] {
        f.file(path, path);
    }
    f.file(".npmignore", "dist\nindex.js\ncli.js\nREADME.md\n");
    f.file("dist/.npmignore", "*.map\n*.txt\n!keep.txt\n");
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.root.join("source.js"), f.root.join("dist/link.js")).unwrap();
    let output = f.ok(&["pack", "--json", "--pack-destination=out"]);
    nojson::RawJson::parse(&output).unwrap();
    let bytes = fs::read(f.root.join("out/team-demo-1.2.3.tgz")).unwrap();
    let files = archive(&bytes);
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "package/LICENSE",
            "package/README.md",
            "package/cli.js",
            "package/dist/index.js",
            "package/dist/keep.txt",
            "package/index.js",
            "package/package.json"
        ]
    );
    assert_eq!(files["package/cli.js"].1, 0o755);
    let json = nojson::RawJson::parse(&output).unwrap();
    let summary = json.value().to_array().unwrap().next().unwrap();
    let integrity: String = summary
        .to_member("integrity")
        .unwrap()
        .required()
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        integrity,
        format!("sha512-{}", STANDARD.encode(Sha512::digest(&bytes)))
    );
    f.ok(&["pack", "--pack-destination=out"]);
    // Exclude the output directory explicitly on this second pack: files already allowlists dist.
    assert_eq!(
        fs::read(f.root.join("out/team-demo-1.2.3.tgz")).unwrap(),
        bytes
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn pack_uses_npmignore_instead_of_gitignore_and_supports_negation() {
    let f = Fixture::new();
    f.file("package.json", r#"{"name":"demo","version":"1.0.0"}"#);
    f.file(".gitignore", "git-only.js\n");
    f.file("git-only.js", "included when npmignore exists");
    f.file("excluded.js", "ignored");
    f.ok(&["pack"]);
    assert!(
        !archive(&fs::read(f.root.join("demo-1.0.0.tgz")).unwrap())
            .contains_key("package/git-only.js")
    );
    f.file(".npmignore", "*.js\n!git-only.js\n");
    f.ok(&["pack"]);
    let files = archive(&fs::read(f.root.join("demo-1.0.0.tgz")).unwrap());
    assert!(files.contains_key("package/git-only.js"));
    assert!(!files.contains_key("package/excluded.js"));
}

#[test]
fn pack_excludes_custom_named_credential_files_even_if_explicitly_allowlisted() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"name":"demo","version":"1.0.0","files":["credentials","client-secret"]}"#,
    );
    f.file(
        "credentials",
        &format!(
            "//{}:_authToken=secret\n//{}:keyfile=${{HOME}}/../client-secret\n",
            f.url.trim_start_matches("http://"),
            f.url.trim_start_matches("http://")
        ),
    );
    f.file("client-secret", "private key bytes");
    f.ok(&["pack", "--userconfig=credentials"]);
    let files = archive(&fs::read(f.root.join("demo-1.0.0.tgz")).unwrap());
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        ["package/package.json"]
    );
}

#[test]
fn dry_run_packs_without_writing_or_contacting_registry_and_keeps_json_clean() {
    let f = Fixture::new();
    f.file("package.json", r#"{"name":"demo","version":"1.0.0"}"#);
    for command in ["pack", "publish"] {
        let out = f.ok(&[command, "--dry-run", "--json", "--offline"]);
        assert!(out.contains("package.json"));
        assert!(out.contains("\"dryRun\": true"));
        nojson::RawJson::parse(&out).unwrap();
    }
    assert!(!f.root.join("demo-1.0.0.tgz").exists());
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn publish_puts_authenticated_metadata_and_integrity_verified_attachment() {
    let f = Fixture::new();
    auth(&f);
    f.file(
        "package.json",
        r#"{"name":"demo","version":"1.2.3","description":"test publication"}"#,
    );
    f.file("index.js", "module.exports = 42;\n");
    f.routes
        .lock()
        .unwrap()
        .insert("/demo".into(), b"{}".to_vec());
    let out = f.ok(&[
        "publish",
        "--tag=beta",
        "--access=public",
        "--otp=654321",
        "--json",
    ]);
    nojson::RawJson::parse(&out).unwrap();
    let writes = f.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    let (method, path, bytes, headers) = &writes[0];
    assert_eq!((method.as_str(), path.as_str()), ("PUT", "/demo"));
    assert!(headers.to_ascii_lowercase().contains("npm-otp: 654321"));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("authorization: bearer publish-secret")
    );
    let text = std::str::from_utf8(bytes).unwrap();
    let data = nojson::RawJson::parse(text).unwrap();
    assert_eq!(string(&data, &["dist-tags", "beta"]), "1.2.3");
    assert_eq!(string(&data, &["access"]), "public");
    let tarball = STANDARD
        .decode(string(&data, &["_attachments", "demo-1.2.3.tgz", "data"]))
        .unwrap();
    let files = archive(&tarball);
    assert!(files.contains_key("package/index.js"));
    assert!(!files.contains_key("package/.npmrc"));
    assert_eq!(
        string(&data, &["versions", "1.2.3", "dist", "integrity"]),
        format!("sha512-{}", STANDARD.encode(Sha512::digest(&tarball)))
    );
    assert_eq!(
        string(&data, &["versions", "1.2.3", "dist", "shasum"]),
        format!("{:x}", sha1::Sha1::digest(&tarball))
    );
    assert_eq!(
        string(&data, &["versions", "1.2.3", "dist", "tarball"]),
        format!("{}demo/-/demo-1.2.3.tgz", f.url)
    );
    assert!(!out.contains("publish-secret"));
    assert!(!f.root.join("demo-1.2.3.tgz").exists());
}

#[test]
fn scoped_publish_config_selects_registry_tag_and_access_but_cli_can_override() {
    let f = Fixture::new();
    let private = Fixture::new();
    f.file("package.json", &format!(r#"{{"name":"@team/demo","version":"1.0.0","publishConfig":{{"registry":"{}team/","tag":"next","access":"restricted"}}}}"#, private.url));
    f.file(
        ".npmrc",
        &format!(
            "//{}team/:_authToken=private-token\n//{}:_authToken=public-token\n",
            private.url.trim_start_matches("http://"),
            f.url.trim_start_matches("http://")
        ),
    );
    private
        .routes
        .lock()
        .unwrap()
        .insert("/team/@team%2Fdemo".into(), b"{}".to_vec());
    f.routes
        .lock()
        .unwrap()
        .insert("/@team%2Fdemo".into(), b"{}".to_vec());
    f.ok(&["publish"]);
    assert!(f.requests.lock().unwrap().is_empty());
    assert_eq!(private.writes.lock().unwrap().len(), 1);
    assert!(
        private
            .authorizations
            .lock()
            .unwrap()
            .iter()
            .any(|(_, auth)| auth.as_deref() == Some("Bearer private-token"))
    );
    let writes = private.writes.lock().unwrap();
    let data = nojson::RawJson::parse(std::str::from_utf8(&writes[0].2).unwrap()).unwrap();
    assert_eq!(string(&data, &["dist-tags", "next"]), "1.0.0");
    assert_eq!(string(&data, &["access"]), "restricted");
    assert!(!string(&data, &["_attachments", "@team/demo-1.0.0.tgz", "data"]).is_empty());
    drop(writes);
    f.ok(&[
        "publish",
        "--registry",
        &f.url,
        "--tag=latest",
        "--access=public",
    ]);
    assert_eq!(f.writes.lock().unwrap().len(), 1);
    assert!(
        f.authorizations
            .lock()
            .unwrap()
            .iter()
            .any(|(_, auth)| auth.as_deref() == Some("Bearer public-token"))
    );
}

#[test]
fn publication_rejects_private_invalid_names_versions_tags_and_unsupported_inputs() {
    let f = Fixture::new();
    for manifest in [
        r#"{"name":"demo","version":"1.0.0","private":true}"#,
        r#"{"name":"../escape","version":"1.0.0"}"#,
        r#"{"name":"demo","version":"v1.0.0"}"#,
        r#"{"name":"demo","version":"1.0.0","publishConfig":{"_authToken":"secret"}}"#,
        r#"{"name":"demo","version":"1.0.0","bundledDependencies":["dep"]}"#,
        r#"{"name":"demo","version":"1.0.0","dependencies":{"dep":"workspace:*"}}"#,
    ] {
        f.file("package.json", manifest);
        assert!(
            !f.invoke(&["publish", "--dry-run"]).status.success(),
            "{manifest}"
        );
    }
    f.file("package.json", r#"{"name":"demo","version":"1.0.0"}"#);
    for args in [
        vec!["publish", "--tag=1.0.0", "--dry-run"],
        vec!["publish", "--tag=..", "--dry-run"],
        vec!["publish", "--access=restricted", "--dry-run"],
        vec!["publish"],
        vec!["publish", "--offline"],
        vec!["publish", "missing.tgz"],
        vec!["publish", "--workspaces"],
        vec!["publish", "--provenance"],
    ] {
        assert!(!f.invoke(&args).status.success(), "{args:?}");
    }
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn publish_refuses_redirects_and_does_not_run_success_hooks_after_registry_failure() {
    let f = Fixture::new();
    let other = Fixture::new();
    auth(&f);
    f.file("package.json", r#"{"name":"demo","version":"1.0.0"}"#);
    f.redirects
        .lock()
        .unwrap()
        .insert("/demo".into(), format!("{}stolen", other.url));
    assert!(!f.invoke(&["publish"]).status.success());
    assert!(other.requests.lock().unwrap().is_empty());
}

#[test]
fn dist_tags_use_scoped_endpoints_with_uncached_list_add_and_remove() {
    let f = Fixture::new();
    auth(&f);
    let path = "/-/package/@team%2Fdemo/dist-tags";
    f.routes.lock().unwrap().insert(
        path.into(),
        br#"{"next":"2.0.0","latest":"1.0.0"}"#.to_vec(),
    );
    f.routes
        .lock()
        .unwrap()
        .insert(format!("{path}/beta"), b"{}".to_vec());
    f.file("package.json", r#"{"name":"@team/demo"}"#);
    assert_eq!(f.ok(&["dist-tag"]), "latest: 1.0.0\nnext: 2.0.0\n");
    let json = f.ok(&["dist-tag", "ls", "@team/demo", "--json"]);
    nojson::RawJson::parse(&json).unwrap();
    f.ok(&["dist-tag", "add", "@team/demo@2.0.0", "beta"]);
    f.ok(&["dist-tag", "rm", "@team/demo", "beta"]);
    let writes = f.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].0, "PUT");
    assert_eq!(writes[0].2, b"\"2.0.0\"");
    assert_eq!(writes[1].0, "DELETE");
    drop(writes);
    let count = f.requests.lock().unwrap().len();
    for args in [
        vec!["dist-tag", "add", "demo@^1", "beta"],
        vec!["dist-tag", "add", "demo@1.0.0", "1"],
        vec!["dist-tag", "rm", "demo", ".."],
        vec!["dist-tag", "add", "demo@1.0.0", "beta", "--offline"],
    ] {
        assert!(!f.invoke(&args).status.success());
    }
    assert_eq!(f.requests.lock().unwrap().len(), count);
}

#[cfg(unix)]
#[test]
fn publish_lifecycle_order_reloads_generated_manifest_and_respects_ignore_scripts() {
    let f = Fixture::new();
    auth(&f);
    f.file("package.json", r#"{"name":"demo","version":"1.0.0","files":["generated.js"],"scripts":{"prepublishOnly":"echo prepublishOnly >> hooks","prepack":"echo prepack >> hooks","prepare":"echo prepare >> hooks; echo built > generated.js","postpack":"echo postpack >> hooks","publish":"echo publish >> hooks","postpublish":"echo postpublish >> hooks"}}"#);
    f.routes
        .lock()
        .unwrap()
        .insert("/demo".into(), b"{}".to_vec());
    f.ok(&["publish", "--json"]);
    assert_eq!(
        fs::read_to_string(f.root.join("hooks")).unwrap(),
        "prepublishOnly\nprepack\nprepare\npostpack\npublish\npostpublish\n"
    );
    let old = fs::read(f.root.join("hooks")).unwrap();
    f.ok(&["publish", "--ignore-scripts", "--json"]);
    assert_eq!(fs::read(f.root.join("hooks")).unwrap(), old);
    f.statuses.lock().unwrap().insert("/demo".into(), 403);
    assert!(!f.invoke(&["publish"]).status.success());
    assert!(
        fs::read_to_string(f.root.join("hooks"))
            .unwrap()
            .ends_with("prepublishOnly\nprepack\nprepare\npostpack\n")
    );
}
