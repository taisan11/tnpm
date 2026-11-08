//! Isolated node_modules layout and shared hardlink store integration tests.
use crate::support::Fixture;
use std::fs;

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
    assert!(store_match, "installed file does not point to shared store inode");

    f.file("second/package.json", r#"{"dependencies":{"app":"1"}}"#);
    let output = f
        .command()
        .args(["--prefix", "second", "install", "--node-linker=isolated", "--offline", "--ignore-scripts"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
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
        assert_ne!(metadata.file_attributes() & 0x400, 0, "{} is not a junction", link.display());
        assert!(link.join("package.json").is_file());
    }
    assert!(f.root.join("home/.tnpm/cache/files-v1").is_dir());
    f.ok(&["ci", "--node-linker=isolated", "--ignore-scripts", "--offline"]);
    assert!(f.root.join("node_modules/app/node_modules/sub/package.json").is_file());
}
