use crate::support::Fixture;
use std::fs;

#[test]
fn import_method_configuration_and_empty_graphs() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    assert_eq!(f.ok(&["get", "package-import-method"]).trim(), "auto");
    for method in ["auto", "hardlink", "copy", "clone-or-copy"] {
        f.file(".npmrc", &format!("package-import-method={method}\n"));
        assert_eq!(f.ok(&["get", "package-import-method"]).trim(), method);
        f.ok(&["install", "--ignore-scripts"]);
    }
    let output = f
        .command()
        .env("npm_config_package_import_method", "copy")
        .args(["get", "package-import-method"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "copy");
    let output = f
        .command()
        .env("npm_config_package_import_method", "copy")
        .args([
            "get",
            "package-import-method",
            "--package-import-method=hardlink",
        ])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "hardlink");
    assert!(
        !f.invoke(&["install", "--package-import-method=invalid"])
            .status
            .success()
    );
    f.file(".npmrc", "package-import-method=invalid\n");
    assert!(!f.invoke(&["install"]).status.success());
}

#[cfg(unix)]
#[test]
fn hoisted_hardlinks_share_cache_but_copy_modes_isolate() {
    use std::os::unix::fs::MetadataExt;
    for method in ["hardlink", "copy", "clone-or-copy"] {
        let f = Fixture::new();
        f.single("app", "");
        f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
        f.file(".npmrc", &format!("package-import-method={method}\n"));
        for pass in 0..2 {
            if pass == 1 {
                fs::remove_dir_all(f.root.join("node_modules")).unwrap();
            }
            f.ok(&["install", "--ignore-scripts"]);
            let project = f.root.join("node_modules/app/index.js");
            let cache = f
                .root
                .join("home/.tnpm/cache/packages-v2/app@1.0.0/package/index.js");
            assert_eq!(
                fs::metadata(&project).unwrap().ino() == fs::metadata(&cache).unwrap().ino(),
                method == "hardlink"
            );
            if method == "hardlink" {
                assert!(fs::metadata(&project).unwrap().nlink() > 1);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn bin_chmod_breaks_the_cache_link_and_keeps_executables_working() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    f.single("app", r#", "bin":{"app":"bin/cli.sh"}"#);
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.file(".npmrc", "package-import-method=hardlink\n");
    f.ok(&["install", "--ignore-scripts"]);
    fs::remove_dir_all(f.root.join("node_modules")).unwrap();
    let cache = f
        .root
        .join("home/.tnpm/cache/packages-v2/app@1.0.0/package/bin/cli.sh");
    fs::set_permissions(&cache, fs::Permissions::from_mode(0o644)).unwrap();
    f.ok(&["install", "--ignore-scripts"]);
    let bin = f.root.join("node_modules/app/bin/cli.sh");
    assert_ne!(
        fs::metadata(&cache).unwrap().ino(),
        fs::metadata(&bin).unwrap().ino()
    );
    assert_eq!(
        fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let output = std::process::Command::new(f.root.join("node_modules/.bin/app"))
        .arg("working")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "working\n");
}

#[test]
fn authorized_hooks_and_rebuild_detach_other_packages_as_well() {
    for rebuild in [false, true] {
        let f = Fixture::new();
        f.single("mutator", r#", "scripts":{"postinstall":"node -e \"require('fs').writeFileSync('../other/index.js','changed')\""}"#);
        f.single("other", "");
        f.file(
            "package.json",
            r#"{"dependencies":{"mutator":"1","other":"1"},"trustedDependencies":["mutator"]}"#,
        );
        f.file(".npmrc", "package-import-method=hardlink\n");
        if rebuild {
            f.ok(&["install", "--ignore-scripts"]);
            f.ok(&["rebuild"]);
        } else {
            f.ok(&["install"]);
        }
        assert_eq!(
            fs::read_to_string(f.root.join("node_modules/other/index.js")).unwrap(),
            "changed"
        );
        assert_eq!(
            fs::read_to_string(
                f.root
                    .join("home/.tnpm/cache/packages-v2/other@1.0.0/package/index.js")
            )
            .unwrap(),
            "module.exports = 42;\n"
        );
    }
}

#[test]
fn root_preinstall_detaches_previously_hardlinked_packages() {
    let f = Fixture::new();
    f.single("app", "");
    f.file(".npmrc", "package-import-method=hardlink\n");
    f.file("package.json", r#"{"dependencies":{"app":"1"}}"#);
    f.ok(&["install", "--ignore-scripts"]);
    f.file("package.json", r#"{"dependencies":{"app":"1"},"scripts":{"preinstall":"node -e \"require('fs').writeFileSync('node_modules/app/index.js','root')\""}}"#);
    f.ok(&["install"]);
    assert_eq!(
        fs::read_to_string(f.root.join("node_modules/app/index.js")).unwrap(),
        "root"
    );
    assert_eq!(
        fs::read_to_string(
            f.root
                .join("home/.tnpm/cache/packages-v2/app@1.0.0/package/index.js")
        )
        .unwrap(),
        "module.exports = 42;\n"
    );
}
