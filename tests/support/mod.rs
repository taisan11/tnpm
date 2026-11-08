//! Isolated project directories, package archives, and a loopback-only registry.
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha512};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

type Authorizations = Arc<Mutex<Vec<(String, Option<String>)>>>;
type Posts = Arc<Mutex<Vec<(String, Vec<u8>)>>>;
type Writes = Arc<Mutex<Vec<(String, String, Vec<u8>, String)>>>;

pub(crate) fn strip_ansi(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut output = String::new();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        } else {
            output.push(c);
        }
    }
    output
}

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) url: String,
    pub(crate) routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    pub(crate) requests: Arc<Mutex<Vec<String>>>,
    pub(crate) authorizations: Authorizations,
    pub(crate) posts: Posts,
    pub(crate) writes: Writes,
    pub(crate) statuses: Arc<Mutex<HashMap<String, u16>>>,
    pub(crate) redirects: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) content_types: Arc<Mutex<HashMap<String, String>>>,
    pub(crate) cache_controls: Arc<Mutex<HashMap<String, String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    pub(crate) fn single(&self, name: &str, extra: &str) {
        self.releases(name, "1.0.0", &[("1.0.0", extra)]);
    }

    pub(crate) fn releases(&self, name: &str, latest: &str, releases: &[(&str, &str)]) {
        let versions: Vec<_> = releases
            .iter()
            .map(|(version, extra)| (*version, self.package(name, version, extra)))
            .collect();
        self.registry(name, latest, &versions);
    }
    pub(crate) fn new() -> Self {
        static ID: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "tnpm-tests-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("home")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let routes = Arc::new(Mutex::new(HashMap::<String, Vec<u8>>::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let authorizations = Arc::new(Mutex::new(Vec::new()));
        let posts = Arc::new(Mutex::new(Vec::new()));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let statuses = Arc::new(Mutex::new(HashMap::<String, u16>::new()));
        let redirects = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let content_types = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let cache_controls = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (server_routes, server_requests, server_stop) =
            (routes.clone(), requests.clone(), stop.clone());
        let server_authorizations = authorizations.clone();
        let server_posts = posts.clone();
        let server_writes = writes.clone();
        let server_statuses = statuses.clone();
        let server_redirects = redirects.clone();
        let server_types = content_types.clone();
        let server_cache = cache_controls.clone();
        let thread = thread::spawn(move || {
            while !server_stop.load(Ordering::Relaxed) {
                let (mut socket, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0u8];
                while socket.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let Some(path) = text.split_whitespace().nth(1) else {
                    continue;
                };
                server_requests.lock().unwrap().push(path.to_owned());
                let length = text
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0; length];
                if socket.read_exact(&mut body).is_err() {
                    continue;
                }
                if matches!(
                    text.split_whitespace().next(),
                    Some("POST" | "PUT" | "DELETE" | "PATCH")
                ) {
                    server_writes.lock().unwrap().push((
                        text.split_whitespace().next().unwrap().into(),
                        path.to_owned(),
                        body.clone(),
                        text.to_string(),
                    ));
                }
                if text.starts_with("POST ") {
                    server_posts.lock().unwrap().push((path.to_owned(), body));
                }
                let authorization = text
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .map(|(_, value)| value.trim().to_owned());
                server_authorizations
                    .lock()
                    .unwrap()
                    .push((path.to_owned(), authorization));
                if let Some(location) = server_redirects.lock().unwrap().get(path) {
                    let _ = write!(
                        socket,
                        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    continue;
                }
                let routes = server_routes.lock().unwrap();
                let body = routes
                    .get(path)
                    .or_else(|| routes.get(&path.replace("%2f", "%2F")));
                let status = if let Some(status) = server_statuses.lock().unwrap().get(path) {
                    format!("{status} Fixture")
                } else if body.is_some() {
                    "200 OK".into()
                } else {
                    "404 Not Found".into()
                };
                let body = body.map(Vec::as_slice).unwrap_or(b"not found");
                let content_type = server_types
                    .lock()
                    .unwrap()
                    .get(path)
                    .map(|t| format!("Content-Type: {t}\r\n"))
                    .unwrap_or_default();
                let cache_control = server_cache
                    .lock()
                    .unwrap()
                    .get(path)
                    .cloned()
                    .unwrap_or_else(|| "max-age=3600".into());
                let _ = write!(
                    socket,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{content_type}Cache-Control: {cache_control}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(body);
            }
        });
        Self {
            root,
            url,
            routes,
            requests,
            authorizations,
            posts,
            writes,
            statuses,
            redirects,
            content_types,
            cache_controls,
            stop,
            thread: Some(thread),
        }
    }

    pub(crate) fn file(&self, name: &str, content: &str) {
        let path = self.root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    pub(crate) fn package(&self, name: &str, version: &str, extra: &str) -> String {
        self.package_files(name, version, extra, &[])
    }

    pub(crate) fn package_files(
        &self,
        name: &str,
        version: &str,
        extra: &str,
        files: &[(&str, &str)],
    ) -> String {
        let manifest = format!(r#"{{"name":"{name}","version":"{version}"{extra}}}"#);
        let mut archive = tar::Builder::new(Vec::new());
        for (path, data) in [
            ("package/package.json", manifest.as_bytes()),
            (
                "package/bin/cli.sh",
                b"#!/bin/sh\nprintf '%s\\n' \"$@\"\n".as_slice(),
            ),
            ("package/index.js", b"module.exports = 42;\n".as_slice()),
        ]
        .into_iter()
        .chain(files.iter().map(|(path, text)| (*path, text.as_bytes())))
        {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            archive.append_data(&mut header, path, data).unwrap();
        }
        let tar = archive.into_inner().unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gzip.write_all(&tar).unwrap();
        let bytes = gzip.finish().unwrap();
        let integrity = STANDARD.encode(Sha512::digest(&bytes));
        let path = format!("/tar/{name}-{version}.tgz");
        let encoded_path =
            reqwest::Url::parse(&format!("{}{path}", self.url.trim_end_matches('/')))
                .unwrap()
                .path()
                .to_owned();
        self.routes.lock().unwrap().insert(encoded_path, bytes);
        format!(
            r#"{{"name":"{name}","version":"{version}"{extra},"dist":{{"tarball":"{}{path}","integrity":"sha512-{integrity}"}}}}"#,
            self.url.trim_end_matches('/')
        )
    }

    pub(crate) fn registry(&self, name: &str, latest: &str, versions: &[(&str, String)]) {
        let versions = versions
            .iter()
            .map(|(version, manifest)| format!("\"{version}\":{manifest}"))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!(
            r#"{{"name":"{name}","dist-tags":{{"latest":"{latest}"}},"versions":{{{versions}}}}}"#
        );
        let mut url = reqwest::Url::parse(&self.url).unwrap();
        url.path_segments_mut().unwrap().pop_if_empty().push(name);
        self.routes
            .lock()
            .unwrap()
            .insert(url.path().to_owned(), body.into_bytes());
    }

    pub(crate) fn invoke(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    #[cfg(unix)]
    pub(crate) fn invoke_tty(&self, args: &[&str], term: &str) -> Output {
        use std::{os::fd::FromRawFd, process::Stdio};
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: 24,
            ws_col: 48,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: all output pointers are valid and the optional termios/name
        // pointers are null. Successful descriptors are immediately owned below.
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::addr_of_mut!(size),
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
        // SAFETY: these descriptors were returned by openpty and are not shared.
        let (mut master, slave) = unsafe {
            libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(slave, libc::F_SETFD, libc::FD_CLOEXEC);
            (fs::File::from_raw_fd(master), fs::File::from_raw_fd(slave))
        };
        let reader = thread::spawn(move || {
            let mut output = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                match master.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => output.extend_from_slice(&buffer[..n]),
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) => panic!("could not read pseudo-terminal: {e}"),
                }
            }
            output
        });
        let mut command = self.command();
        command
            .args(args)
            .env("TERM", term)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(slave));
        let child = command.spawn().unwrap();
        drop(command);
        let mut output = child.wait_with_output().unwrap();
        output.stderr = reader.join().unwrap();
        output
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tnpm"));
        command
            .current_dir(&self.root)
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("npm_config_registry", &self.url)
            .env("npm_config_userconfig", self.root.join("absent.npmrc"))
            .env(
                "npm_config_globalconfig",
                self.root.join("absent-global.npmrc"),
            )
            .env_remove("NODE_ENV");
        command
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CLICOLOR")
            .env_remove("NO_COLOR");
        command
    }

    pub(crate) fn ok(&self, args: &[&str]) -> String {
        let out = self.invoke(args);
        assert!(
            out.status.success(),
            "{args:?}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://").trim_end_matches('/'));
        self.thread.take().unwrap().join().unwrap();
        let _ = fs::remove_dir_all(&self.root);
    }
}
