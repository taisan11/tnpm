use crate::support::Fixture;
use sha2::{Digest, Sha256};
use std::fs;

fn module(f: &Fixture, path: &str, source: &str) {
    f.routes
        .lock()
        .unwrap()
        .insert(path.into(), source.as_bytes().to_vec());
}

#[test]
fn remote_typescript_resolves_relative_reexports_dynamic_imports_and_redirects() {
    let f = Fixture::new();
    module(
        &f,
        "/lib/main.ts",
        "export { value } from './value.ts'; export async function extra() { return (await import('./extra.ts')).extra; } export const location = import.meta.url;",
    );
    module(&f, "/lib/value.ts", "export const value: number = 40;");
    module(&f, "/lib/extra.ts", "export const extra: number = 2;");
    f.redirects
        .lock()
        .unwrap()
        .insert("/redirect.ts".into(), format!("{}lib/main.ts", f.url));
    f.file("entry.mjs", &format!("import {{value,extra,location}} from '{}redirect.ts'; console.log(value + await extra());console.log(location);", f.url));
    let out = f.ok(&["run", "entry.mjs"]);
    assert!(out.starts_with("42\n"), "{out}");
    assert!(out.contains(&format!("{}lib/main.ts", f.url)), "{out}");
    assert!(
        f.requests
            .lock()
            .unwrap()
            .iter()
            .any(|p| p == "/lib/value.ts")
    );
}

#[test]
fn commonjs_typescript_entries_can_import_remote_esm_synchronously() {
    let f = Fixture::new();
    module(&f, "/value.ts", "export const value: number = 42;");
    f.file(
        "entry.ts",
        &format!(
            "import {{value}} from '{}value.ts';console.log(value);",
            f.url
        ),
    );
    assert_eq!(f.ok(&["run", "entry.ts"]), "42\n");
}

#[test]
fn fragments_are_distinct_module_instances_but_share_downloaded_source() {
    let f = Fixture::new();
    module(
        &f,
        "/counter.js",
        "globalThis.remoteCounter = (globalThis.remoteCounter || 0) + 1;export const count = globalThis.remoteCounter;export const location = import.meta.url;",
    );
    f.file("entry.mjs", &format!("import {{count as one}} from '{}counter.js#one';import {{count as two, location}} from '{}counter.js#two';console.log(one,two,location);", f.url, f.url));
    assert_eq!(
        f.ok(&["run", "entry.mjs"]),
        format!("1 2 {}counter.js#two\n", f.url)
    );
    assert_eq!(f.requests.lock().unwrap().as_slice(), &["/counter.js"]);
}

#[test]
fn remote_cache_supports_offline_reload_queries_and_corruption_detection() {
    let f = Fixture::new();
    module(&f, "/value.js?one", "export default 1;");
    module(&f, "/value.js?two", "export default 2;");
    f.file("entry.mjs", &format!("import one from '{}value.js?one';import two from '{}value.js?two';console.log(one + two);", f.url, f.url));
    assert!(
        !f.invoke(&["run", "--offline", "entry.mjs"])
            .status
            .success()
    );
    assert!(f.requests.lock().unwrap().is_empty());
    assert_eq!(f.ok(&["run", "entry.mjs"]), "3\n");
    let requests = f.requests.lock().unwrap().len();
    module(&f, "/value.js?one", "export default 10;");
    assert_eq!(f.ok(&["run", "--offline", "entry.mjs"]), "3\n");
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    assert_eq!(f.ok(&["run", "--no-cache", "entry.mjs"]), "12\n");
    // Reload does not mutate the old cache snapshot.
    assert_eq!(f.ok(&["run", "--offline", "entry.mjs"]), "3\n");
    let cache = f.root.join("home/.tnpm/cache/modules-v1");
    for entry in fs::read_dir(cache).unwrap() {
        fs::write(entry.unwrap().path(), "{}").unwrap();
    }
    let before = f.requests.lock().unwrap().len();
    assert!(
        !f.invoke(&["run", "--offline", "entry.mjs"])
            .status
            .success()
    );
    assert_eq!(f.requests.lock().unwrap().len(), before);
    assert_eq!(f.ok(&["run", "entry.mjs"]), "12\n");
}

#[test]
fn no_store_modules_are_not_persisted_and_conflicting_cache_flags_fail_before_fetch() {
    let f = Fixture::new();
    module(&f, "/value.js", "export const value = 42;");
    f.cache_controls
        .lock()
        .unwrap()
        .insert("/value.js".into(), "private, no-store".into());
    f.file(
        "entry.mjs",
        &format!(
            "import {{value}} from '{}value.js';console.log(value);",
            f.url
        ),
    );
    assert_eq!(f.ok(&["run", "entry.mjs"]), "42\n");
    assert!(!f.root.join("home/.tnpm/cache/modules-v1").exists());
    let before = f.requests.lock().unwrap().len();
    assert!(
        !f.invoke(&["run", "--offline", "entry.mjs"])
            .status
            .success()
    );
    assert!(
        !f.invoke(&["run", "--offline", "--no-cache", "entry.mjs"])
            .status
            .success()
    );
    assert_eq!(f.requests.lock().unwrap().len(), before);
}

#[test]
fn oversized_modules_are_rejected_before_execution() {
    let f = Fixture::new();
    f.routes
        .lock()
        .unwrap()
        .insert("/large.js".into(), vec![b' '; 16 * 1024 * 1024 + 1]);
    f.file("entry.mjs", &format!("import '{}large.js';", f.url));
    let out = f.invoke(&["run", "entry.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("exceeds 16 MiB"));
    assert!(!f.root.join("home/.tnpm/cache/modules-v1").exists());
}

#[test]
fn extensionless_and_json_modules_respect_media_types_and_import_attributes() {
    let f = Fixture::new();
    module(&f, "/module", "export const value: number = 40;");
    f.content_types.lock().unwrap().insert(
        "/module".into(),
        "application/typescript; charset=utf-8".into(),
    );
    module(&f, "/data.json", "{\"value\":2}");
    f.file("entry.mjs", &format!("import {{value}} from '{}module';import data from '{}data.json' with {{type:'json'}};console.log(value+data.value);", f.url, f.url));
    assert_eq!(f.ok(&["run", "entry.mjs"]), "42\n");
    f.file(
        "bad.mjs",
        &format!("import data from '{}data.json';", f.url),
    );
    let out = f.invoke(&["run", "bad.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Remote JSON imports require"));
}

#[test]
fn remote_imports_reject_credentials_html_invalid_utf8_and_parse_errors() {
    let f = Fixture::new();
    let url = f.url.replacen("http://", "http://user:secret@", 1);
    f.file("entry.mjs", &format!("import '{url}value.js';"));
    assert!(!f.invoke(&["run", "entry.mjs"]).status.success());
    assert!(f.requests.lock().unwrap().is_empty());
    for (path, source) in [
        ("/html.js", "<html></html>"),
        ("/bad.ts", "const value: = ;"),
    ] {
        module(&f, path, source);
    }
    f.content_types
        .lock()
        .unwrap()
        .insert("/html.js".into(), "text/html".into());
    f.routes
        .lock()
        .unwrap()
        .insert("/bad-utf8.js".into(), vec![0xff]);
    for path in ["html.js", "bad.ts", "bad-utf8.js", "missing.js"] {
        f.file("entry.mjs", &format!("import '{}{path}';", f.url));
        assert!(!f.invoke(&["run", "entry.mjs"]).status.success(), "{path}");
    }
    assert!(!f.root.join("home/.tnpm/cache/modules-v1").exists());
}

fn cache(f: &Fixture, url: &str, final_url: &str, source: &str) {
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let value = nojson::json(|f| {
        f.object(|f| {
            f.member("requested", url)?;
            f.member("url", final_url)?;
            f.member("source", source)?;
            f.member("media", "text/javascript")?;
            f.member("checksum", digest.as_str())
        })
    })
    .to_string();
    let key = format!("{:x}", Sha256::digest(url.as_bytes()));
    f.file(&format!("home/.tnpm/cache/modules-v1/{key}.json"), &value);
}

#[test]
fn https_modules_cannot_downgrade_or_import_local_files() {
    let f = Fixture::new();
    let url = "https://example.test/value.js";
    f.file("entry.mjs", &format!("import '{url}';"));
    cache(&f, url, url, &format!("import '{}leak.js';", f.url));
    let out = f.invoke(&["run", "--offline", "entry.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot downgrade"));
    f.file("local.mjs", "console.log('local-should-not-execute');");
    let local = reqwest::Url::from_file_path(f.root.join("local.mjs")).unwrap();
    cache(&f, url, url, &format!("import '{local}';"));
    let out = f.invoke(&["run", "--offline", "entry.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot import local file URLs"));
    cache(&f, url, &format!("{}value.js", f.url), "export default 42;");
    assert!(
        !f.invoke(&["run", "--offline", "entry.mjs"])
            .status
            .success()
    );
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn authenticated_remote_modules_do_not_persist_or_forward_scoped_credentials() {
    let f = Fixture::new();
    let other = Fixture::new();
    module(
        &f,
        "/private/main.js",
        "export {value} from '/public/value.js';",
    );
    module(&f, "/public/value.js", "export const value = 42;");
    f.file(
        ".npmrc",
        &format!(
            "//{}private/:_authToken=secret\n",
            f.url.strip_prefix("http://").unwrap()
        ),
    );
    f.file(
        "entry.mjs",
        &format!(
            "import {{value}} from '{}private/main.js';console.log(value);",
            f.url
        ),
    );
    assert_eq!(f.ok(&["run", "entry.mjs"]), "42\n");
    for (path, auth) in f.authorizations.lock().unwrap().iter() {
        assert_eq!(
            auth.as_deref(),
            if path.starts_with("/private/") {
                Some("Bearer secret")
            } else {
                None
            }
        );
    }
    let requests = f.requests.lock().unwrap().len();
    assert!(
        !f.invoke(&["run", "--offline", "entry.mjs"])
            .status
            .success()
    );
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    f.redirects
        .lock()
        .unwrap()
        .insert("/private/main.js".into(), format!("{}leak.js", other.url));
    assert!(!f.invoke(&["run", "entry.mjs"]).status.success());
    assert!(other.requests.lock().unwrap().is_empty());
}

fn jsr_packages(f: &Fixture) {
    f.file(".npmrc", &format!("@jsr:registry={}\n", f.url));
    let child = f.package_files(
        "@jsr/demo__child",
        "1.0.0",
        r#", "type":"module","exports":"./mod.js""#,
        &[("package/mod.js", "export const value = 40;")],
    );
    f.registry("@jsr/demo__child", "1.0.0", &[("1.0.0", child)]);
    let math = f.package_files("@jsr/demo__math", "1.0.0", r#", "type":"module","exports":{".":"./mod.js","./extra":"./extra.js"},"dependencies":{"@jsr/demo__child":"^1"},"scripts":{"install":"echo should-not-run > installed"}"#,
        &[("package/mod.js", "export {value} from '@jsr/demo__child';"), ("package/extra.js", "export const extra = 2;")]);
    f.registry("@jsr/demo__math", "1.0.0", &[("1.0.0", math)]);
}

#[test]
fn jsr_runtime_imports_resolve_exports_transitives_and_offline_caches_without_project_edits() {
    let f = Fixture::new();
    jsr_packages(&f);
    f.file("package.json", r#"{"name":"project","type":"module"}"#);
    let before = fs::read(f.root.join("package.json")).unwrap();
    f.file("entry.mjs", "import {value} from 'jsr:@demo/math@1';import {extra} from 'jsr:@demo/math@1/extra';console.log(value+extra);");
    assert_eq!(f.ok(&["run", "entry.mjs"]), "42\n");
    assert_eq!(fs::read(f.root.join("package.json")).unwrap(), before);
    assert!(!f.root.join("tnpm-lock.json").exists());
    assert!(!f.root.join("node_modules").exists());
    let requests = f.requests.lock().unwrap().len();
    assert_eq!(f.ok(&["run", "--offline", "entry.mjs"]), "42\n");
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    for cache in fs::read_dir(f.root.join("home/.tnpm/exec")).unwrap() {
        assert!(
            !cache
                .unwrap()
                .path()
                .join("node_modules/@demo/math/installed")
                .exists()
        );
    }
}

#[test]
fn jsr_protocol_imports_reuse_matching_installed_aliases_without_network() {
    let f = Fixture::new();
    jsr_packages(&f);
    f.ok(&["add", "jsr:@demo/math@1", "--ignore-scripts"]);
    f.file(
        "tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@demo/math":["./fake.mjs"]}}}"#,
    );
    f.file("fake.mjs", "export const value = 0;");
    f.file(
        "entry.mjs",
        "import {value} from 'jsr:@demo/math@1';console.log(value);",
    );
    let requests = f.requests.lock().unwrap().len();
    assert_eq!(f.ok(&["run", "--offline", "entry.mjs"]), "40\n");
    assert_eq!(f.requests.lock().unwrap().len(), requests);
    assert!(!f.root.join("home/.tnpm/exec").exists());
}

#[test]
fn remote_modules_can_import_jsr_and_npm_protocol_dependencies() {
    let f = Fixture::new();
    jsr_packages(&f);
    let plain = f.package_files(
        "plain",
        "1.0.0",
        r#", "type":"module","exports":"./mod.js""#,
        &[("package/mod.js", "export const extra = 2;")],
    );
    f.registry("plain", "1.0.0", &[("1.0.0", plain)]);
    module(
        &f,
        "/main.ts",
        "import {value} from 'jsr:@demo/math@1';import {extra} from 'npm:plain@1';export const total: number = value+extra;",
    );
    f.file(
        "entry.mjs",
        &format!(
            "import {{total}} from '{}main.ts';console.log(total);",
            f.url
        ),
    );
    assert_eq!(f.ok(&["run", "entry.mjs"]), "42\n");
    let requests = f.requests.lock().unwrap().len();
    assert_eq!(f.ok(&["run", "--offline", "entry.mjs"]), "42\n");
    assert_eq!(f.requests.lock().unwrap().len(), requests);
}

#[test]
fn remote_bare_imports_invalid_jsr_subpaths_and_bad_package_integrity_fail_clearly() {
    let f = Fixture::new();
    jsr_packages(&f);
    module(&f, "/bad.js", "import 'bare-package';");
    f.file("entry.mjs", &format!("import '{}bad.js';", f.url));
    let out = f.invoke(&["run", "entry.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("require an npm: or jsr:"));
    f.file("entry.mjs", "import 'jsr:@demo/math@1/../secret';");
    let before = f.requests.lock().unwrap().len();
    assert!(!f.invoke(&["run", "entry.mjs"]).status.success());
    assert_eq!(f.requests.lock().unwrap().len(), before);
    f.routes.lock().unwrap().insert(
        "/tar/@jsr/demo__math-1.0.0.tgz".into(),
        b"tampered".to_vec(),
    );
    f.file("entry.mjs", "import 'jsr:@demo/math@1';");
    let out = f.invoke(&["run", "entry.mjs"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("EINTEGRITY"));
}
