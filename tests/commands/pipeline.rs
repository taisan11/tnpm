use crate::support::Fixture;
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[test]
fn streaming_discovery_and_archive_preparation_overlap_without_changing_placement() {
    let f = Fixture::new();
    f.single("seed", r#", "dependencies":{"shared":"1"}"#);
    f.single("shared", "");
    f.file("package.json", r#"{"dependencies":{"seed":"1"}}"#);
    f.ok(&["install", "--ignore-scripts", "--no-cache"]);
    f.single("a", r#", "dependencies":{"child":"1"}"#);
    f.single("z", r#", "dependencies":{"shared":"2"}"#);
    f.single("slow", "");
    f.single("marker", "");
    f.single("child", r#", "dependencies":{"shared":"1"}"#);
    f.releases(
        "shared",
        "2.0.0",
        &[
            ("1.0.0", ""),
            ("1.1.0", r#", "dependencies":{"marker":"1"}"#),
            ("2.0.0", ""),
        ],
    );
    f.file(
        "package.json",
        r#"{"dependencies":{"a":"1","slow":"1","z":"1"}}"#,
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let registry = format!("http://{}/", listener.local_addr().unwrap());
    f.file(
        "tnpm-lock.json",
        &fs::read_to_string(f.root.join("tnpm-lock.json"))
            .unwrap()
            .replace(&f.url, &registry),
    );
    let routes = f
        .routes
        .lock()
        .unwrap()
        .iter()
        .map(|(path, bytes)| {
            let bytes = if path.starts_with("/tar/") {
                bytes.clone()
            } else {
                String::from_utf8(bytes.clone())
                    .unwrap()
                    .replace(&f.url, &registry)
                    .into_bytes()
            };
            (path.clone(), bytes)
        })
        .collect::<std::collections::HashMap<_, _>>();
    let routes = Arc::new(routes);
    let events = Arc::new((Mutex::new(Vec::<(String, String)>::new()), Condvar::new()));
    let failed_gate = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (server_events, server_failed, server_stop) =
        (events.clone(), failed_gate.clone(), stop.clone());
    let server = thread::spawn(move || {
        let mut workers = Vec::new();
        while !server_stop.load(Ordering::Relaxed) {
            let (mut socket, _) = match listener.accept() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) => panic!("{error}"),
            };
            let (events, failed, routes) =
                (server_events.clone(), server_failed.clone(), routes.clone());
            workers.push(thread::spawn(move || {
                socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0];
                while socket.read(&mut byte).unwrap_or(0) == 1 {
                    bytes.push(byte[0]);
                    if bytes.ends_with(b"\r\n\r\n") { break; }
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let (log, ready) = &*events;
                let mut log = log.lock().unwrap();
                log.push((path.clone(), request));
                ready.notify_all();
                let wait_for = match path.as_str() {
                    "/z" => Some("/child"),
                    "/child" => Some("/tar/a-1.0.0.tgz"),
                    "/slow" => Some("/marker"),
                    _ => None,
                };
                if let Some(expected) = wait_for {
                    let (guard, result) = ready.wait_timeout_while(log, Duration::from_secs(5), |log| !log.iter().any(|(p, _)| p == expected)).unwrap();
                    log = guard;
                    if result.timed_out() { failed.store(true, Ordering::Relaxed); }
                }
                drop(log);
                let (status, body) = routes.get(&path).map(|b| (200, b.as_slice())).unwrap_or((404, b"{}"));
                let header = format!("HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nCache-Control: public, max-age=600\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(header.as_bytes()).and_then(|_| socket.write_all(body));
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });
    let run = |args: &[&str]| {
        f.command()
            .env("npm_config_registry", &registry)
            .args(args)
            .output()
            .unwrap()
    };
    let installed = run(&["install", "--ignore-scripts"]);
    let viewed = run(&["view", "a", "version"]);
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(
        installed.status.success(),
        "{}",
        String::from_utf8_lossy(&installed.stderr)
    );
    assert!(
        viewed.status.success(),
        "{}",
        String::from_utf8_lossy(&viewed.stderr)
    );
    assert!(
        !failed_gate.load(Ordering::Relaxed),
        "metadata or tarball waited for a resolution wave"
    );
    let (events, _) = &*events;
    let events = events.lock().unwrap();
    assert_eq!(events.iter().filter(|(p, _)| p == "/shared").count(), 1);
    assert_eq!(
        events.iter().filter(|(p, _)| p == "/a").count(),
        2,
        "view must not reuse abbreviated metadata"
    );
    assert!(
        events
            .iter()
            .any(|(p, h)| p == "/a" && h.contains("application/vnd.npm.install-v1+json"))
    );
    assert!(
        events
            .iter()
            .any(|(p, h)| p == "/a" && h.contains("application/json"))
    );
    for archive in [
        "a-1.0.0",
        "z-1.0.0",
        "slow-1.0.0",
        "child-1.0.0",
        "shared-1.0.0",
        "shared-2.0.0",
    ] {
        assert_eq!(
            events
                .iter()
                .filter(|(p, _)| p == &format!("/tar/{archive}.tgz"))
                .count(),
            1
        );
    }
    assert!(
        fs::read_to_string(f.root.join("node_modules/shared/package.json"))
            .unwrap()
            .contains("2.0.0")
    );
    assert!(
        fs::read_to_string(
            f.root
                .join("node_modules/child/node_modules/shared/package.json")
        )
        .unwrap()
        .contains("1.0.0")
    );
    assert!(!f.root.join("node_modules/marker").exists());
    assert!(
        !events
            .iter()
            .any(|(p, _)| p == "/tar/shared-1.1.0.tgz" || p == "/tar/marker-1.0.0.tgz")
    );
}

#[test]
fn lock_only_and_empty_graphs_do_not_prefetch_archives() {
    let f = Fixture::new();
    f.single("app", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"app":"1"},"optionalDependencies":{"missing":"1"}}"#,
    );
    f.ok(&["install", "--package-lock-only", "--ignore-scripts"]);
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .all(|p| !p.starts_with("/tar/"))
    );
    assert!(!f.root.join("node_modules").exists());
    f.file("package.json", "{}");
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .all(|p| !p.starts_with("/tar/"))
    );
}

#[test]
fn failed_prefetch_integrity_never_replaces_installed_files() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file("node_modules/keep/index.js", "preserved");
    f.routes
        .lock()
        .unwrap()
        .insert("/tar/app-1.0.0.tgz".into(), b"corrupt archive".to_vec());
    let output = f.invoke(&["install", "--ignore-scripts"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("EINTEGRITY"));
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/keep/index.js")).unwrap(),
        "preserved"
    );
    assert!(
        !f.root
            .join("home/.tnpm/cache/packages-v2/app@1.0.0/.complete")
            .exists()
    );
    f.file("package.json", r#"{"optionalDependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/app").exists());
}

#[test]
fn full_metadata_cache_still_supports_offline_resolution() {
    let f = Fixture::new();
    f.single("app", "");
    f.ok(&["view", "app", "version"]);
    let requests = f.requests.lock().unwrap().len();
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&[
        "install",
        "--package-lock-only",
        "--offline",
        "--ignore-scripts",
    ]);
    assert_eq!(requests, f.requests.lock().unwrap().len());
    assert!(f.root.join("tnpm-lock.json").is_file());
}

#[test]
fn offline_resolution_uses_full_metadata_when_abbreviated_versions_are_stale() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--package-lock-only"]);
    f.releases("app", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.ok(&["view", "app", "version"]);
    let requests = f.requests.lock().unwrap().len();
    f.file("package.json", r#"{"dependencies":{"app":"2"}}"#);
    f.ok(&["install", "--package-lock-only", "--offline"]);
    assert!(
        fs::read_to_string(f.root.join("tnpm-lock.json"))
            .unwrap()
            .contains("2.0.0")
    );
    f.file("package.json", r#"{"dependencies":{"app":"3"}}"#);
    assert!(
        !f.invoke(&["install", "--package-lock-only", "--offline"])
            .status
            .success()
    );
    assert_eq!(requests, f.requests.lock().unwrap().len());
}

#[cfg(unix)]
#[test]
fn hidden_lock_timestamp_ties_do_not_hide_manifest_edits() {
    let f = Fixture::new();
    f.file(".npmrc", "package-import-method=copy\n");
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    let modified = fs::metadata(f.root.join("node_modules/.tnpm-lock.json"))
        .unwrap()
        .modified()
        .unwrap();
    f.file(
        "node_modules/app/package.json",
        r#"{"name":"app","version":"0.0.0"}"#,
    );
    f.file("node_modules/app/new-file", "changes directory mtime");
    fs::File::open(f.root.join("node_modules/app"))
        .unwrap()
        .set_modified(modified)
        .unwrap();
    f.ok(&["install", "--ignore-scripts"]);
    assert!(
        fs::read_to_string(f.root.join("node_modules/app/package.json"))
            .unwrap()
            .contains("1.0.0")
    );
}

#[test]
fn isolated_index_restoration_does_not_prefetch_unused_payloads() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    fs::remove_dir_all(f.root.join("home/.tnpm/cache/packages-v2")).unwrap();
    fs::remove_file(f.root.join("tnpm-lock.json")).unwrap();
    let requests = f.requests.lock().unwrap().len();
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    assert_eq!(requests, f.requests.lock().unwrap().len());
    assert!(!f.root.join("home/.tnpm/cache/packages-v2").exists());
    assert!(f.root.join("node_modules/app/package.json").is_file());
}
