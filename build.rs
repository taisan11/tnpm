use std::{path::PathBuf, process::Command};

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn main() {
    println!("cargo::rerun-if-env-changed=TNPM_TSRS_BUNDLE");
    println!("cargo::rustc-check-cfg=cfg(tnpm_bundled_tsrs)");
    if let Some(bundle) = std::env::var_os("TNPM_TSRS_BUNDLE") {
        let bundle = PathBuf::from(bundle);
        assert!(bundle.is_file(), "TNPM_TSRS_BUNDLE must point to a tar.gz");
        println!("cargo::rerun-if-changed={}", bundle.display());
        println!("cargo::rustc-env=TNPM_TSRS_BUNDLE={}", bundle.canonicalize().unwrap().display());
        println!("cargo::rustc-env=TNPM_TSRS_TARGET={}", std::env::var("TARGET").unwrap());
        println!("cargo::rustc-cfg=tnpm_bundled_tsrs");
    }
    println!("cargo::rerun-if-changed=build.rs");
    // Resolve paths through Git so linked worktrees and packed refs also work.
    for name in ["HEAD", "refs", "packed-refs"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            let path = PathBuf::from(path);
            if path.exists() {
                println!("cargo::rerun-if-changed={}", path.display());
            }
        }
    }
    let commit = git(&["rev-parse", "--short=7", "HEAD"])
        .filter(|hash| !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo::rustc-env=TNPM_BUILD_COMMIT={commit}");
}
