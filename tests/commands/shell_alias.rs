use crate::support::Fixture;
use std::fs;

#[test]
fn creates_and_removes_only_its_own_block() {
    for (shell, file) in [
        ("bash", ".bashrc"),
        ("zsh", ".zshrc"),
        ("fish", ".config/fish/config.fish"),
    ] {
        let f = Fixture::new();
        let path = f.root.join("home").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "alias npm='original'";
        fs::write(&path, original).unwrap();
        let run = |action: &str| {
            f.command()
                .env("SHELL", format!("/bin/{shell}"))
                .env_remove("ZDOTDIR")
                .env_remove("XDG_CONFIG_HOME")
                .args(["alias", action])
                .output()
                .unwrap()
        };
        assert!(run("create").status.success());
        let created = fs::read_to_string(&path).unwrap();
        assert!(created.contains("alias npm='tnpm'"));
        assert!(run("create").status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), created);
        assert!(run("delete").status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(run("delete").status.success());
    }
}

#[test]
fn rejects_modified_blocks_and_invalid_commands() {
    let f = Fixture::new();
    f.file(
        "home/.bashrc",
        "# >>> tnpm npm alias >>>\nalias npm='other'\n# <<< tnpm npm alias <<<\n",
    );
    let before = fs::read_to_string(f.root.join("home/.bashrc")).unwrap();
    let output = f
        .command()
        .env("SHELL", "/bin/bash")
        .args(["alias", "delete"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(f.root.join("home/.bashrc")).unwrap(),
        before
    );
    assert!(!f.invoke(&["alias"]).status.success());
    assert!(!f.invoke(&["alias", "create", "extra"]).status.success());
    assert!(
        !f.command()
            .env("SHELL", "/bin/unknown")
            .args(["alias", "create"])
            .output()
            .unwrap()
            .status
            .success()
    );
}
