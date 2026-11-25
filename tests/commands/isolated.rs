//! Isolated node_modules layout and shared hardlink store integration tests.
use crate::support::Fixture;
use std::fs;

#[test]
fn corrupt_installed_manifest_does_not_block_repair_or_removal() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    let install = ["install", "--node-linker=isolated", "--ignore-scripts"];
    let path = "node_modules/app/package.json";
    let installed = f.root.join(path);
    f.ok(&install);
    let corrupt_manifest = || {
        fs::remove_file(&installed).unwrap();
        f.file(path, "{");
    };
    corrupt_manifest();
    assert_eq!(f.ok(&install), "+ app@1.0.0\n\ninstalled 1 package\n");
    let repaired = fs::read_to_string(&installed).unwrap();
    nojson::RawJson::parse(&repaired).unwrap();
    corrupt_manifest();
    f.ok(&[
        "remove",
        "app",
        "--node-linker=isolated",
        "--ignore-scripts",
    ]);
    assert!(!installed.exists());
}

#[cfg(unix)]
#[test]
fn indexed_install_works_without_extracted_cache_and_preserves_bins() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    f.single(
        "app",
        r#", "bin":{"app":"bin/cli.sh"}, "dependencies":{"sub":"1"}"#,
    );
    f.single("sub", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    let original = fs::metadata(f.root.join("node_modules/app/index.js")).unwrap();
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    fs::remove_dir_all(f.root.join("home/.tnpm/cache/packages-v2")).unwrap();
    let requests = f.requests.lock().unwrap().len();
    let output = f.invoke(&["install", "--node-linker=isolated", "--offline"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(f.root.join("node_modules/app/index.js"))
            .unwrap()
            .ino(),
        original.ino()
    );
    fs::remove_file(f.root.join("tnpm-lock.json")).unwrap();
    let output = f.invoke(&["install", "--node-linker=isolated", "--offline"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::metadata(f.root.join("node_modules/app/index.js"))
            .unwrap()
            .ino(),
        original.ino()
    );
    assert_eq!(requests, f.requests.lock().unwrap().len());
    f.file("package.json", r#"{"devDependencies":{"app":"1"}}"#);
    let output = f.invoke(&["install", "--node-linker=isolated", "--offline"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed = fs::metadata(f.root.join("node_modules/app/index.js")).unwrap();
    assert_eq!(original.ino(), installed.ino());
    assert!(installed.nlink() >= 2);
    assert!(
        f.root
            .join("node_modules/app/node_modules/sub/package.json")
            .is_file()
    );
    assert!(!f.root.join("node_modules/sub").exists());
    assert!(
        fs::metadata(f.root.join("node_modules/.bin/app"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111
            != 0
    );
    assert!(
        fs::read_dir(f.root.join("home/.tnpm/cache/tmp-v1"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn missing_and_corrupt_indexed_objects_are_repaired_without_replacing_other_objects() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    let index_dir = f.root.join("home/.tnpm/cache/files-index-v1");
    let index_path = fs::read_dir(&index_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(index_path).unwrap();
    let index = nojson::RawJson::parse(&text).unwrap();
    let object: String = index
        .value()
        .to_member("files")
        .unwrap()
        .required()
        .unwrap()
        .to_member("index.js")
        .unwrap()
        .required()
        .unwrap()
        .to_member("object")
        .unwrap()
        .required()
        .unwrap()
        .try_into()
        .unwrap();
    let object = f
        .root
        .join("home/.tnpm/cache/files-v1")
        .join(&object[..2])
        .join(&object);
    let manifest = f.root.join("node_modules/app/package.json");
    let untouched = fs::metadata(&manifest).unwrap().ino();
    fs::remove_file(&object).unwrap();
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert_eq!(fs::metadata(&manifest).unwrap().ino(), untouched);
    fs::set_permissions(&object, fs::Permissions::from_mode(0o644)).unwrap();
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert_eq!(
        fs::metadata(&object).unwrap().permissions().mode() & 0o222,
        0
    );
    assert_eq!(fs::metadata(&manifest).unwrap().ino(), untouched);
    fs::set_permissions(&object, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&object, b"corrupted contents\n").unwrap();
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/index.js")).unwrap(),
        "module.exports = 42;\n"
    );
    assert_eq!(fs::metadata(&manifest).unwrap().ino(), untouched);
}

#[cfg(unix)]
#[test]
fn isolated_hooks_run_before_linking_even_when_pristine_indices_exist() {
    let f = Fixture::new();
    f.single(
        "app",
        r#", "dependencies":{"sub":"1"}, "scripts":{"install":"echo modified > ../sub/index.js"}"#,
    );
    f.single("sub", "");
    f.file(
        "package.json",
        r#"{"dependencies":{"app":"1"},"trustedDependencies":["app"]}"#,
    );
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    f.ok(&["install", "--node-linker=isolated", "--offline"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/node_modules/sub/index.js")).unwrap(),
        "modified\n"
    );
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/node_modules/sub/index.js")).unwrap(),
        "module.exports = 42;\n"
    );
}

#[test]
fn isolated_empty_graph_and_invalid_index_paths() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    let path = fs::read_dir(f.root.join("home/.tnpm/cache/files-index-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("\"index.js\":", "\"../escape\":");
    fs::write(path, text).unwrap();
    let output = f.invoke(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsafe package index path"));
    assert!(!f.root.join("escape").exists());
}

#[cfg(unix)]
#[test]
fn indexed_store_symlinks_are_rejected_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    let index_path = fs::read_dir(f.root.join("home/.tnpm/cache/files-index-v1"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(index_path).unwrap();
    let index = nojson::RawJson::parse(&text).unwrap();
    let object: String = index
        .value()
        .to_member("files")
        .unwrap()
        .required()
        .unwrap()
        .to_member("index.js")
        .unwrap()
        .required()
        .unwrap()
        .to_member("object")
        .unwrap()
        .required()
        .unwrap()
        .try_into()
        .unwrap();
    let object = f
        .root
        .join("home/.tnpm/cache/files-v1")
        .join(&object[..2])
        .join(&object);
    fs::remove_file(&object).unwrap();
    f.file("untouched", "original");
    symlink(f.root.join("untouched"), &object).unwrap();
    let output = f.invoke(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("symlink"));
    assert_eq!(
        fs::read_to_string(f.root.join("untouched")).unwrap(),
        "original"
    );
}

#[cfg(unix)]
#[test]
fn root_hooks_still_run_before_hardlink_conversion() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"postinstall":"echo root-built > node_modules/app/index.js"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    f.ok(&["install", "--node-linker=isolated", "--offline"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/index.js")).unwrap(),
        "root-built\n"
    );
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/index.js")).unwrap(),
        "module.exports = 42;\n"
    );
}

#[cfg(unix)]
#[test]
fn isolated_layout_resolves_conflicting_transitive_versions_without_exposing_them() {
    let f = Fixture::new();
    f.releases("shared", "2.0.0", &[("1.0.0", ""), ("2.0.0", "")]);
    f.single("left", r#", "dependencies":{"shared":"1"}"#);
    f.single("right", r#", "dependencies":{"shared":"2"}"#);
    f.file(
        "package.json",
        r#"{"dependencies":{"left":"1","right":"1"}}"#,
    );
    f.file(".npmrc", "node-linker=isolated\n");
    f.ok(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    assert!(!f.root.join("node_modules/shared").exists());
    for (name, expected) in [("left", "1.0.0"), ("right", "2.0.0")] {
        let path = f.root.join(format!(
            "node_modules/{name}/node_modules/shared/package.json"
        ));
        let data = fs::read_to_string(&path).unwrap();
        assert!(
            data.contains(&format!("\"version\":\"{expected}\"")),
            "{}",
            path.display()
        );
    }
    f.ok(&["install", "--ignore-scripts", "--offline"]);
    f.ok(&["ci", "--ignore-scripts", "--offline"]);
    assert!(
        f.root
            .join("node_modules/left/node_modules/shared/package.json")
            .is_file()
    );
    f.ok(&["ls", "--all"]);
}

#[cfg(unix)]
#[test]
fn isolated_linker_can_switch_to_hoisted_and_back() {
    let f = Fixture::new();
    f.single("app", r#", "dependencies":{"sub":"1"}"#);
    f.single("sub", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/sub").exists());
    f.ok(&["install", "--node-linker=hoisted", "--ignore-scripts"]);
    assert!(!f.root.join("node_modules/.tnpm-layout").exists());
    assert!(f.root.join("node_modules/sub/package.json").is_file());
    f.ok(&[
        "install",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert!(
        f.root
            .join("node_modules/app/node_modules/sub/package.json")
            .is_file()
    );
}

#[cfg(unix)]
#[test]
fn store_dir_and_user_configuration_are_shared_across_projects() {
    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file(
        "home/.npmrc",
        "node-linker=isolated\nstore-dir=~/shared-tnpm-store\n",
    );
    let run = |args: &[&str]| {
        let output = f
            .command()
            .args(args)
            .env_remove("npm_config_userconfig")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let output = run(&["get", "node-linker"]);
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "isolated\n");
    run(&["install", "--ignore-scripts"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    assert!(
        f.root
            .join("home/shared-tnpm-store/packages-v2/app@1.0.0/.complete")
            .is_file()
    );
    f.file(".npmrc", "node-linker=hoisted\n");
    assert_eq!(
        String::from_utf8(run(&["get", "node-linker"]).stdout).unwrap(),
        "hoisted\n"
    );
    run(&["install", "--ignore-scripts", "--node-linker=isolated"]);
    assert!(f.root.join("node_modules/.tnpm-layout").is_file());
    let bad = f.invoke(&["install", "--node-linker=invalid"]);
    assert!(!bad.status.success());
}

#[cfg(unix)]
#[test]
fn project_files_are_hardlinked_to_global_content_store() {
    use std::os::unix::fs::MetadataExt;

    let f = Fixture::new();
    f.single("app", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);

    let file = f.root.join("node_modules/app/index.js");
    let original = fs::metadata(&file).unwrap();
    assert!(original.nlink() >= 2, "installed file is not hardlinked");
    let objects = f.root.join("home/.tnpm/cache/files-v1");
    let mut store_match = false;
    for bucket in fs::read_dir(&objects).unwrap() {
        for entry in fs::read_dir(bucket.unwrap().path()).unwrap() {
            let metadata = fs::metadata(entry.unwrap().path()).unwrap();
            if metadata.dev() == original.dev() && metadata.ino() == original.ino() {
                store_match = true;
            }
        }
    }
    assert!(
        store_match,
        "installed file does not point to shared store inode"
    );

    f.file("second/package.json", r#"{"dependencies":{"app":"1"}}"#);
    let output = f
        .command()
        .args([
            "--prefix",
            "second",
            "install",
            "--node-linker=isolated",
            "--offline",
            "--ignore-scripts",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let second = fs::metadata(f.root.join("second/node_modules/app/index.js")).unwrap();
    assert_eq!(original.dev(), second.dev());
    assert_eq!(original.ino(), second.ino());
}

#[cfg(windows)]
#[test]
fn windows_uses_directory_junctions_and_hardlinked_payloads() {
    use std::os::windows::fs::MetadataExt;

    let f = Fixture::new();
    f.single("app", r#", "dependencies":{"sub":"1"}"#);
    f.single("sub", "");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--node-linker=isolated", "--ignore-scripts"]);

    for path in ["node_modules/app", "node_modules/app/node_modules/sub"] {
        let link = f.root.join(path);
        let metadata = fs::symlink_metadata(&link).unwrap();
        assert_ne!(
            metadata.file_attributes() & 0x400,
            0,
            "{} is not a junction",
            link.display()
        );
        assert!(link.join("package.json").is_file());
    }
    assert!(f.root.join("home/.tnpm/cache/files-v1").is_dir());
    f.ok(&[
        "ci",
        "--node-linker=isolated",
        "--ignore-scripts",
        "--offline",
    ]);
    assert!(
        f.root
            .join("node_modules/app/node_modules/sub/package.json")
            .is_file()
    );
}
