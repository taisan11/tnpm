//! Direct entry execution and its interaction with package scripts.
use crate::support::Fixture;

#[test]
fn runs_typescript_and_resolves_imports_and_arguments() {
    let f = Fixture::new();
    f.file("package.json", r#"{"type":"module"}"#);
    f.file(
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    f.file("src/value.ts", "export const value: number = 42;");
    f.file("src/entry.ts", "import { value } from '@/value'; console.log(JSON.stringify([value, ...process.argv.slice(2)]));");
    let output = f.ok(&["run", "src/entry.ts", "a b", "--flag", "$(unsafe)"]);
    assert!(
        output.contains(r#"[42,"a b","--flag","$(unsafe)"]"#),
        "{output}"
    );
}

#[test]
fn script_names_take_priority_over_files() {
    let f = Fixture::new();
    f.file(
        "package.json",
        r#"{"scripts":{"entry.js":"node -e \"console.log('script')\""}}"#,
    );
    f.file("entry.js", "throw new Error('file should not execute');");
    assert!(f.ok(&["run", "entry.js"]).contains("script"));
}

#[test]
fn javascript_and_commonjs_use_the_resolver() {
    let f = Fixture::new();
    f.file(
        "tsconfig.json",
        r#"{"compilerOptions":{"baseUrl":".","paths":{"@/*":["src/*"]}}}"#,
    );
    f.file("src/value.cts", "module.exports = 41 as number;");
    f.file("entry.cjs", "console.log(require('@/value.cts') + 1);");
    assert!(f.ok(&["run", "entry.cjs"]).contains("42"));
}

#[test]
fn commonjs_typescript_transforms_imports_exports_and_reexports() {
    let f = Fixture::new();
    f.file(
        "value.cts",
        "export const value: number = 40; export default 2;",
    );
    f.file("reexport.cts", "export * from './value.cts';");
    f.file("entry.cts", "import extra from './value.cts'; import { value } from './reexport.cts'; console.log(value + extra);");
    assert_eq!(f.ok(&["run", "entry.cts"]), "42\n");
    f.file("entry.cjs", "console.log(require('./value.cts').value);");
    assert_eq!(f.ok(&["run", "entry.cjs"]), "40\n");
    f.file(
        "entry.ts",
        "import extra from './value.cts'; console.log(extra);",
    );
    assert_eq!(f.ok(&["run", "entry.ts"]), "2\n");
}

#[test]
fn esm_javascript_resolves_ts_alias_and_js_extension() {
    let f = Fixture::new();
    f.file("package.json", r#"{"type":"module"}"#);
    f.file(
        "tsconfig.json",
        r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
    );
    f.file("src/value.ts", "export const value: number = 42;");
    f.file(
        "entry.js",
        "import { value } from '@/value.js'; console.log(value);",
    );
    assert!(f.ok(&["run", "entry.js"]).contains("42"));
}

#[test]
fn missing_entry_fails_without_starting_node() {
    let f = Fixture::new();
    let output = f.invoke(&["run", "missing.ts"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("runner entry not found"));
}

#[test]
fn runner_propagates_exit_code() {
    let f = Fixture::new();
    f.file("entry.mjs", "process.exit(23);");
    assert_eq!(f.invoke(&["run", "entry.mjs"]).status.code(), Some(23));
}

#[test]
fn reports_invalid_typescript() {
    let f = Fixture::new();
    f.file("package.json", r#"{"type":"module"}"#);
    f.file("broken.ts", "const value: = ;");
    let output = f.invoke(&["run", "broken.ts"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("broken.ts"));
}
