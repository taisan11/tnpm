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

#[test]
fn commonjs_preserves_live_bindings_and_default_interop() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file(
        "dep.cts",
        r#"export default 40;
export let counter = 0;
export function bump() { counter++; }"#,
    );
    f.file(
        "entry.cts",
        r#"import value, { counter, bump } from "./dep.cts";
import * as namespace from "./dep.cts";
bump();
console.log(JSON.stringify([value, counter, namespace.counter]));"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "[40,1,1]\n");
}

#[test]
fn commonjs_shorthand_and_callee_imports_keep_esm_semantics() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file(
        "dep.cts",
        r#"export let value = 1;
export function setValue(next: number) { value = next; }
export function receiver() { return typeof this; }"#,
    );
    f.file(
        "entry.cts",
        r#"import { value, setValue, receiver } from "./dep.cts";
const snapshot = { value };
setValue(2);
console.log(JSON.stringify([snapshot.value, value, receiver()]));"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "[1,2,\"undefined\"]\n");
}

#[test]
fn commonjs_supports_reexports_namespaces_and_default_declarations() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file(
        "dep.cts",
        r#"export const value = 1;
export default function named() { return 2; }"#,
    );
    f.file(
        "mid.cts",
        r#"export * from "./dep.cts";
export { default as named } from "./dep.cts";
export * as dependency from "./dep.cts";"#,
    );
    f.file(
        "entry.cts",
        r#"import { value, named, dependency } from "./mid.cts";
console.log(JSON.stringify([value, named(), dependency.value, dependency.default()]));"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "[1,2,1,2]\n");
}

#[test]
fn commonjs_local_exports_win_over_star_exports() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file("dep.cts", r#"export const shared = "dep";"#);
    f.file(
        "mid.cts",
        r#"export * from "./dep.cts";
export const shared = "local";"#,
    );
    f.file(
        "entry.cts",
        r#"import { shared } from "./mid.cts";
console.log(shared);"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "local\n");
}

#[test]
fn commonjs_supports_anonymous_default_function_and_class() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file("fn.cts", "export default function () { return 3; }");
    f.file(
        "class.cts",
        "export default class { constructor() { this.value = 5; } }",
    );
    f.file(
        "entry.cts",
        r#"import make from "./fn.cts";
import Value from "./class.cts";
console.log(make() + new Value().value);"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "8\n");
}

#[test]
fn commonjs_typescript_export_assignment_round_trips() {
    let f = Fixture::new();
    f.file("package.json", "{}");
    f.file("value.cts", "const value: number = 42; export = value;");
    f.file(
        "entry.cts",
        r#"import value = require("./value.cts");
console.log(value);"#,
    );
    assert_eq!(f.ok(&["run", "entry.cts"]), "42\n");
}
