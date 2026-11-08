use crate::support::Fixture;
use std::fs;

#[test]
fn bare_and_named_tarball_urls_are_pinned_and_reinstalled_offline() {
    let f = Fixture::new();
    f.package("remote", "1.0.0", "");
    let url = format!("{}tar/remote-1.0.0.tgz", f.url);
    f.ok(&["add", &url, "--ignore-scripts"]);
    let manifest = fs::read_to_string(f.root.join("package.json")).unwrap();
    assert!(manifest.contains(&url));
    let lock = fs::read(f.root.join("tnpm-lock.json")).unwrap();
    assert!(String::from_utf8_lossy(&lock).contains("sha512-"));
    assert_eq!(
        f.requests.lock().unwrap().as_slice(),
        &["/tar/remote-1.0.0.tgz"]
    );
    let before = f.requests.lock().unwrap().len();
    // Exercise the pinned source-archive cache rather than the extracted-package cache.
    fs::remove_dir_all(f.root.join("home/.tnpm/cache/packages-v2")).unwrap();
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/remote/package.json").is_file());
    f.ok(&["outdated"]);
    assert_eq!(f.requests.lock().unwrap().len(), before);
    assert_eq!(fs::read(f.root.join("tnpm-lock.json")).unwrap(), lock);
    // Named specs preserve installation aliases; no live registry is involved.
    f.ok(&["add", &format!("alias@{url}"), "--ignore-scripts"]);
    assert!(f.root.join("node_modules/alias/package.json").is_file());
    f.ok(&["ci", "--offline", "--ignore-scripts"]);
}

#[test]
fn remote_sources_include_transitive_urls_and_respect_authentication() {
    let f = Fixture::new();
    f.package("child", "1.0.0", "");
    let child = format!("{}tar/child-1.0.0.tgz", f.url);
    f.package(
        "parent",
        "1.0.0",
        &format!(r#", "dependencies":{{"child":"{child}"}}"#),
    );
    f.file(
        ".npmrc",
        &format!(
            "//{}/:_authToken=tar-secret\n",
            f.url.trim_start_matches("http://").trim_end_matches('/')
        ),
    );
    let url = format!("{}tar/parent-1.0.0.tgz", f.url);
    f.ok(&["add", &url, "--ignore-scripts"]);
    assert!(f.root.join("node_modules/child/package.json").is_file());
    assert!(
        f.authorizations
            .lock()
            .unwrap()
            .iter()
            .all(|(_, auth)| auth.as_deref() == Some("Bearer tar-secret"))
    );
    // Authenticated package bytes are not persisted to shared caches.
    assert!(
        !f.invoke(&["ci", "--offline", "--ignore-scripts"])
            .status
            .success()
    );
}

#[test]
fn pinned_url_integrity_is_checked_before_replacing_any_packages() {
    let f = Fixture::new();
    f.package("remote", "1.0.0", "");
    let url = format!("{}tar/remote-1.0.0.tgz", f.url);
    f.ok(&["add", &url, "--package-lock-only", "--ignore-scripts"]);
    f.file("node_modules/keep.txt", "keep");
    f.package("remote", "1.0.0", r#", "description":"tampered""#);
    let out = f.invoke(&["install", "--no-cache", "--ignore-scripts"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("EINTEGRITY"));
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/keep.txt")).unwrap(),
        "keep"
    );
    assert!(!f.root.join("node_modules/remote").exists());
}

#[test]
fn rejects_url_credentials_unsafe_archives_and_relative_remote_file_dependencies() {
    let f = Fixture::new();
    let host = f.url.trim_start_matches("http://");
    assert!(
        !f.invoke(&["add", &format!("http://user:secret@{host}bad.tgz")])
            .status
            .success()
    );
    assert!(f.requests.lock().unwrap().is_empty());
    let mut header = tar::Header::new_gnu();
    header.set_path("package/escape").unwrap();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_link_name("../../outside").unwrap();
    header.set_size(0);
    header.set_cksum();
    let mut tar = tar::Builder::new(Vec::new());
    tar.append(&header, &[][..]).unwrap();
    f.routes
        .lock()
        .unwrap()
        .insert("/bad.tar".into(), tar.into_inner().unwrap());
    assert!(
        !f.invoke(&["add", &format!("{}bad.tar", f.url)])
            .status
            .success()
    );
    assert!(!f.root.join("outside").exists());
    f.package(
        "remote",
        "1.0.0",
        r#", "dependencies":{"local":"file:../local"}"#,
    );
    let out = f.invoke(&["add", &format!("{}tar/remote-1.0.0.tgz", f.url)]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("relative file dependencies"));
}
