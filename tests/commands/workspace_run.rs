use crate::support::Fixture;
use std::fs;

#[test]
fn topological_execution_orders_runtime_and_dev_dependencies_before_dependents() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/*"]}"#);
    f.file("packages/a/package.json", r#"{"name":"a","version":"1.0.0","dependencies":{"b":"workspace:*"},"scripts":{"build":"node -e \"if(!require('fs').existsSync('../b/done'))process.exit(8);require('fs').writeFileSync('done','a')\""}}"#);
    f.file("packages/b/package.json", r#"{"name":"b","version":"1.0.0","devDependencies":{"c":"^0"},"scripts":{"build":"node -e \"if(!require('fs').existsSync('../c/done'))process.exit(9);require('fs').writeFileSync('done','b')\""}}"#);
    f.file(
        "packages/c/package.json",
        r#"{"name":"c","scripts":{"build":"node -e \"require('fs').writeFileSync('done','c')\""}}"#,
    );
    f.ok(&[
        "run",
        "--workspaces",
        "--parallel",
        "--topological",
        "--concurrency",
        "2",
        "build",
    ]);
    for name in ["a", "b", "c"] {
        assert!(f.root.join(format!("packages/{name}/done")).is_file());
    }
    assert!(f.requests.lock().unwrap().is_empty());
}

#[test]
fn parallel_workspaces_overlap_and_forward_arguments() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/*"]}"#);
    f.file("barrier.cjs", "const fs=require('fs');const [self,other,arg]=process.argv.slice(2);fs.writeFileSync('ready',arg);const start=Date.now();function check(){if(fs.existsSync('../'+other+'/ready')){fs.writeFileSync('done',arg);return}if(Date.now()-start>10000)process.exit(9);setTimeout(check,10)}check();");
    for (name, other) in [("a", "b"), ("b", "a")] {
        f.file(&format!("packages/{name}/package.json"), &format!(r#"{{"name":"{name}","scripts":{{"check":"node ../../barrier.cjs {name} {other}"}}}}"#));
    }
    f.ok(&[
        "run",
        "--workspaces",
        "--parallel",
        "--concurrency=2",
        "check",
        "--",
        "forwarded",
    ]);
    for name in ["a", "b"] {
        assert_eq!(
            fs::read_to_string(f.root.join(format!("packages/{name}/done"))).unwrap(),
            "forwarded"
        );
    }
}

#[test]
fn dependency_cycles_are_rejected_before_any_script_starts() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/*"]}"#);
    for (name, other) in [("a", "b"), ("b", "a")] {
        f.file(&format!("packages/{name}/package.json"), &format!(r#"{{"name":"{name}","dependencies":{{"{other}":"workspace:*"}},"scripts":{{"build":"echo bad > ran"}}}}"#));
    }
    let out = f.invoke(&["run", "--workspaces", "--topological", "build"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("dependency cycle"));
    assert!(!f.root.join("packages/a/ran").exists());
    assert!(!f.root.join("packages/b/ran").exists());
    // Explicitly selecting a single member does not implicitly add dependencies.
    f.ok(&["run", "-w", "a", "--topological", "build"]);
}

#[test]
fn workspace_failures_stop_dependents_and_keep_exit_codes_and_existing_order() {
    let f = Fixture::new();
    f.file("package.json", r#"{"workspaces":["packages/*"]}"#);
    f.file(
        "packages/a/package.json",
        r#"{"name":"a","dependencies":{"b":"workspace:*"},"scripts":{"build":"echo bad > ran"}}"#,
    );
    f.file(
        "packages/b/package.json",
        r#"{"name":"b","scripts":{"build":"exit 7"}}"#,
    );
    assert_eq!(
        f.invoke(&["run", "--workspaces", "--topological", "build"])
            .status
            .code(),
        Some(7)
    );
    assert!(!f.root.join("packages/a/ran").exists());
    f.ok(&[
        "run",
        "--workspaces",
        "--parallel",
        "--if-present",
        "missing",
    ]);
    assert!(
        !f.invoke(&["run", "--workspaces", "--concurrency=0", "build"])
            .status
            .success()
    );
    assert!(!f.invoke(&["run", "--parallel", "build"]).status.success());
    // No flags retains the previous declaration/discovery order, not dependency order.
    assert_eq!(
        f.invoke(&["run", "--workspaces", "build"]).status.code(),
        Some(7)
    );
    assert!(f.root.join("packages/a/ran").is_file());
}
