//! Lower ECMAScript modules to CommonJS for Node's CommonJS loader.
//!
//! The parser provides statement spans and semantic references, so the
//! transform rewrites only module declarations while leaving the rest of the
//! lowered source untouched. Imports are hoisted as `require` calls, imported
//! bindings are replaced with property reads to retain live bindings, and
//! exported bindings are exposed through getters.
use anyhow::{Context, Result, bail};
use oxc_allocator::Allocator;
use oxc_ast::{AstKind, ast::*};
use oxc_parser::Parser;
use oxc_semantic::{Semantic, SemanticBuilder};
use oxc_span::{GetSpan, SourceType};
use std::{collections::HashMap, fmt::Write as _, path::Path};

pub fn transform(path: &Path, source: String) -> Result<String> {
    lower(&source).with_context(|| format!("could not lower {} to CommonJS", path.display()))
}

fn lower(source: &str) -> Result<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::default().with_module(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        let diagnostics = parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic
                    .render_with_source_code(source.to_owned())
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!("failed to parse CommonJS output:\n{diagnostics}");
    }
    let semantic = SemanticBuilder::new()
        .with_build_nodes(true)
        .with_check_syntax_error(true)
        .build(&parsed.program);
    if !semantic.diagnostics.is_empty() {
        let diagnostics = semantic
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic
                    .render_with_source_code(source.to_owned())
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!("failed to analyze CommonJS output:\n{diagnostics}");
    }

    let body_start = parsed
        .program
        .body
        .first()
        .map_or(source.len(), |statement| statement.span().start as usize);
    let mut lowering = Lowering::new(source);
    for statement in &parsed.program.body {
        match statement {
            Statement::ImportDeclaration(declaration) => {
                lowering.import(declaration, &semantic.semantic);
            }
            Statement::ExportFromDeclaration(declaration) => lowering.require_export(declaration),
            Statement::ExportAllDeclaration(declaration) => {
                lowering.require_export_all(declaration)
            }
            _ => {}
        }
    }
    for statement in &parsed.program.body {
        match statement {
            Statement::ExportDeclaration(declaration) => {
                lowering.export_declaration(&declaration.declaration);
            }
            Statement::ExportNamedDeclaration(declaration) => lowering.export_named(declaration),
            Statement::ExportFromDeclaration(declaration) => lowering.export_from(declaration),
            Statement::ExportAllDeclaration(declaration) => lowering.export_all(declaration),
            Statement::ExportDefaultDeclaration(declaration) => {
                lowering.export_default(&declaration.declaration);
            }
            _ => {}
        }
    }

    lowering.rewrites.sort_by_key(|rewrite| rewrite.start);
    let mut body = String::new();
    let mut cursor = body_start;
    for statement in &parsed.program.body {
        let start = statement.span().start as usize;
        let end = statement.span().end as usize;
        body.push_str(&lowering.render(cursor, start));
        match statement {
            Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_) => body.push(';'),
            Statement::ExportDeclaration(declaration) => {
                body.push_str(&lowering.render(
                    declaration.declaration.span().start as usize,
                    declaration.declaration.span().end as usize,
                ));
            }
            Statement::ExportDefaultDeclaration(declaration) => {
                lowering.render_default(&declaration.declaration, &mut body);
            }
            _ => body.push_str(&lowering.render(start, end)),
        }
        cursor = end;
    }
    body.push_str(&lowering.render(cursor, source.len()));

    let header = lowering.finish();
    let mut output = String::with_capacity(source.len() + header.len());
    output.push_str(&source[..body_start]);
    output.push_str(&header);
    output.push_str(&body);
    Ok(output)
}

#[derive(Clone)]
struct Rewrite {
    start: usize,
    end: usize,
    text: String,
}

struct Lowering<'a> {
    source: &'a str,
    imports: String,
    exports: String,
    star_exports: String,
    helpers: Helpers,
    has_exports: bool,
    temp: usize,
    exports_from: HashMap<usize, String>,
    import_exprs: HashMap<String, String>,
    rewrites: Vec<Rewrite>,
}

#[derive(Default)]
struct Helpers {
    default_interop: bool,
    wildcard_interop: bool,
    export_star: bool,
}

impl<'a> Lowering<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            imports: String::new(),
            exports: String::new(),
            star_exports: String::new(),
            helpers: Helpers::default(),
            has_exports: false,
            temp: 0,
            exports_from: HashMap::new(),
            import_exprs: HashMap::new(),
            rewrites: Vec::new(),
        }
    }

    fn finish(self) -> String {
        let mut output = String::new();
        if self.helpers.default_interop {
            output.push_str(
                "function _tnpm$interopDefault(module) { return module && module.__esModule ? module : { default: module }; }\n",
            );
        }
        if self.helpers.wildcard_interop {
            output.push_str(
                "function _tnpm$interopWildcard(module) {\n\
                 \x20 if (module && module.__esModule) return module;\n\
                 \x20 const namespace = { default: module };\n\
                 \x20 if (module != null) { for (const key in module) { if (key !== \"default\") namespace[key] = module[key]; } }\n\
                 \x20 return namespace;\n\
                 }\n",
            );
        }
        if self.helpers.export_star {
            output.push_str(
                "function _tnpm$exportStar(target, module) {\n\
                 \x20 for (const key of Object.keys(module)) {\n\
                 \x20   if (key !== \"default\" && key !== \"__esModule\" && !Object.prototype.hasOwnProperty.call(target, key)) {\n\
                 \x20     Object.defineProperty(target, key, { enumerable: true, get: function () { return module[key]; } });\n\
                 \x20   }\n\
                 \x20 }\n\
                 }\n",
            );
        }
        if self.has_exports {
            output.push_str("Object.defineProperty(exports, \"__esModule\", { value: true });\n");
        }
        output.push_str(&self.imports);
        output.push_str(&self.exports);
        output.push_str(&self.star_exports);
        output
    }

    fn temp(&mut self) -> String {
        loop {
            let name = format!("_tnpm$m{}", self.temp);
            self.temp += 1;
            if !self.source.contains(&name) {
                return name;
            }
        }
    }

    fn import(&mut self, declaration: &ImportDeclaration<'_>, semantic: &Semantic<'_>) {
        if declaration.import_kind == ImportOrExportKind::Type {
            return;
        }
        let source = specifier(&declaration.source);
        let Some(specifiers) = declaration.specifiers.as_ref().filter(|s| !s.is_empty()) else {
            let _ = writeln!(self.imports, "require({source});");
            return;
        };
        let module = self.temp();
        let _ = writeln!(self.imports, "const {module} = require({source});");
        for specifier in specifiers {
            match specifier {
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    self.helpers.default_interop = true;
                    let expression = format!("_tnpm$interopDefault({module}).default");
                    self.bind(&specifier.local, expression, semantic);
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    self.helpers.wildcard_interop = true;
                    let local = specifier.local.name.to_string();
                    let _ = writeln!(
                        self.imports,
                        "const {local} = _tnpm$interopWildcard({module});"
                    );
                    self.import_exprs
                        .insert(local, specifier.local.name.to_string());
                }
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    if specifier.import_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    let imported = specifier.imported.name();
                    let expression = if imported.as_str() == "default" {
                        self.helpers.default_interop = true;
                        format!("_tnpm$interopDefault({module}).default")
                    } else {
                        member(&module, imported.as_str())
                    };
                    self.bind(&specifier.local, expression, semantic);
                }
            }
        }
    }

    fn bind(&mut self, local: &BindingIdentifier<'_>, expression: String, semantic: &Semantic<'_>) {
        let name = local.name.to_string();
        let Some(symbol_id) = local.symbol_id.get() else {
            // Semantic analysis always resolves import bindings, but fall back
            // to a snapshot rather than leaving an undefined identifier.
            let _ = writeln!(self.imports, "const {name} = {expression};");
            return;
        };
        for reference in semantic.scoping().get_resolved_references(symbol_id) {
            let node = semantic.nodes().get_node(reference.node_id());
            let span = node.span();
            let replacement = match semantic.nodes().parent_kind(reference.node_id()) {
                AstKind::ObjectProperty(property) if property.shorthand => {
                    format!("{name}: {expression}")
                }
                AstKind::CallExpression(call) if call.callee.span() == span => {
                    format!("(0, {expression})")
                }
                AstKind::NewExpression(new) if new.callee.span() == span => {
                    format!("({expression})")
                }
                AstKind::TaggedTemplateExpression(tagged) if tagged.tag.span() == span => {
                    format!("(0, {expression})")
                }
                _ => expression.clone(),
            };
            self.rewrites.push(Rewrite {
                start: span.start as usize,
                end: span.end as usize,
                text: replacement,
            });
        }
        self.import_exprs.insert(name, expression);
    }

    fn require_export(&mut self, declaration: &ExportFromDeclaration<'_>) {
        if declaration.export_kind == ImportOrExportKind::Type {
            return;
        }
        let module = self.temp();
        let source = specifier(&declaration.source);
        let _ = writeln!(self.imports, "const {module} = require({source});");
        self.exports_from
            .insert(declaration.span().start as usize, module);
    }

    fn require_export_all(&mut self, declaration: &ExportAllDeclaration<'_>) {
        if declaration.export_kind == ImportOrExportKind::Type {
            return;
        }
        let module = self.temp();
        let source = specifier(&declaration.source);
        let _ = writeln!(self.imports, "const {module} = require({source});");
        self.exports_from
            .insert(declaration.span().start as usize, module);
    }

    fn export_declaration(&mut self, declaration: &Declaration<'_>) {
        let mut names = Vec::new();
        declaration_names(declaration, &mut names);
        for name in names {
            let value = self.value(&name);
            self.getter(&name, &value);
        }
    }

    fn export_named(&mut self, declaration: &ExportNamedDeclaration<'_>) {
        if declaration.export_kind == ImportOrExportKind::Type {
            return;
        }
        for specifier in &declaration.specifiers {
            if specifier.export_kind == ImportOrExportKind::Type {
                continue;
            }
            let local = specifier.local.name();
            let exported = specifier.exported.name();
            let value = self.value(local.as_str());
            self.getter(exported.as_str(), &value);
        }
    }

    fn export_from(&mut self, declaration: &ExportFromDeclaration<'_>) {
        if declaration.export_kind == ImportOrExportKind::Type {
            return;
        }
        let Some(module) = self
            .exports_from
            .get(&(declaration.span().start as usize))
            .cloned()
        else {
            return;
        };
        for specifier in &declaration.specifiers {
            if specifier.export_kind == ImportOrExportKind::Type {
                continue;
            }
            let local = specifier.local.name();
            let exported = specifier.exported.name();
            let value = if local.as_str() == "default" {
                self.helpers.default_interop = true;
                format!("_tnpm$interopDefault({module}).default")
            } else {
                member(&module, local.as_str())
            };
            self.getter(exported.as_str(), &value);
        }
    }

    fn export_all(&mut self, declaration: &ExportAllDeclaration<'_>) {
        if declaration.export_kind == ImportOrExportKind::Type {
            return;
        }
        let Some(module) = self
            .exports_from
            .get(&(declaration.span().start as usize))
            .cloned()
        else {
            return;
        };
        if let Some(exported) = &declaration.exported {
            self.helpers.wildcard_interop = true;
            self.has_exports = true;
            let name = exported.name();
            let _ = writeln!(
                self.exports,
                "exports[{}] = _tnpm$interopWildcard({module});",
                js_string(name.as_str())
            );
        } else {
            self.helpers.export_star = true;
            self.has_exports = true;
            let _ = writeln!(self.star_exports, "_tnpm$exportStar(exports, {module});");
        }
    }

    fn export_default(&mut self, declaration: &ExportDefaultDeclarationKind<'_>) {
        self.has_exports = true;
        match declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if let Some(id) = &function.id {
                    let name = id.name.to_string();
                    self.getter("default", &name);
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    let name = id.name.to_string();
                    self.getter("default", &name);
                }
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
            _ => {}
        }
    }

    fn render_default(&self, declaration: &ExportDefaultDeclarationKind<'_>, body: &mut String) {
        let span = declaration.span();
        let text = self.render(span.start as usize, span.end as usize);
        match declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                if function.id.is_some() {
                    body.push_str(&text);
                    body.push('\n');
                } else {
                    let _ = writeln!(body, "exports.default = {text};");
                }
            }
            ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                if class.id.is_some() {
                    body.push_str(&text);
                    body.push('\n');
                } else {
                    let _ = writeln!(body, "exports.default = {text};");
                }
            }
            ExportDefaultDeclarationKind::TSInterfaceDeclaration(_) => {}
            _ => {
                let _ = writeln!(body, "exports.default = {text};");
            }
        }
    }

    fn value(&self, local: &str) -> String {
        self.import_exprs
            .get(local)
            .cloned()
            .unwrap_or_else(|| local.to_owned())
    }

    fn getter(&mut self, exported: &str, value: &str) {
        self.has_exports = true;
        let _ = writeln!(
            self.exports,
            "Object.defineProperty(exports, {}, {{ enumerable: true, get: function () {{ return {value}; }} }});",
            js_string(exported)
        );
    }

    fn render(&self, start: usize, end: usize) -> String {
        let mut output = String::new();
        let mut cursor = start;
        for rewrite in self
            .rewrites
            .iter()
            .filter(|rewrite| rewrite.start >= start && rewrite.end <= end)
        {
            output.push_str(&self.source[cursor..rewrite.start]);
            output.push_str(&rewrite.text);
            cursor = rewrite.end;
        }
        output.push_str(&self.source[cursor..end]);
        output
    }
}

fn declaration_names(declaration: &Declaration<'_>, names: &mut Vec<String>) {
    match declaration {
        Declaration::VariableDeclaration(declaration) => {
            for declarator in &declaration.declarations {
                binding_names(&declarator.id, names);
            }
        }
        Declaration::FunctionDeclaration(function) => {
            if let Some(id) = &function.id {
                names.push(id.name.to_string());
            }
        }
        Declaration::ClassDeclaration(class) => {
            if let Some(id) = &class.id {
                names.push(id.name.to_string());
            }
        }
        _ => {}
    }
}

fn binding_names(pattern: &BindingPattern<'_>, names: &mut Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => names.push(id.name.to_string()),
        BindingPattern::ObjectPattern(pattern) => {
            for property in &pattern.properties {
                binding_names(&property.value, names);
            }
            if let Some(rest) = &pattern.rest {
                binding_names(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(pattern) => {
            for element in pattern.elements.iter().flatten() {
                binding_names(element, names);
            }
            if let Some(rest) = &pattern.rest {
                binding_names(&rest.argument, names);
            }
        }
        BindingPattern::AssignmentPattern(pattern) => binding_names(&pattern.left, names),
    }
}

fn specifier(literal: &StringLiteral<'_>) -> String {
    match &literal.raw {
        Some(raw) => raw.to_string(),
        None => js_string(literal.value.as_str().unwrap_or_default()),
    }
}

fn member(base: &str, name: &str) -> String {
    if is_identifier_name(name) {
        format!("{base}.{name}")
    } else {
        format!("{base}[{}]", js_string(name))
    }
}

fn is_identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c == '$' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

fn js_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            c if (c as u32) < 0x20 => {
                let _ = write!(output, "\\u{:04x}", c as u32);
            }
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lower_ok(source: &str) -> String {
        lower(source).unwrap()
    }

    #[test]
    fn lowers_default_named_and_namespace_imports() {
        let output = lower_ok(
            r#"import value, { named as local } from "dep";
import * as ns from "other";
import "side";
console.log(value, local, ns, side);"#,
        );
        assert!(
            output.contains("const _tnpm$m0 = require(\"dep\");"),
            "{output}"
        );
        assert!(
            output.contains("console.log(_tnpm$interopDefault(_tnpm$m0).default"),
            "{output}"
        );
        assert!(output.contains("_tnpm$m0.named"), "{output}");
        assert!(
            output.contains("const ns = _tnpm$interopWildcard(_tnpm$m1);"),
            "{output}"
        );
        assert!(output.contains("require(\"side\");"), "{output}");
        assert!(!output.contains("import "), "{output}");
    }

    #[test]
    fn lowers_exports_and_reexports() {
        let output = lower_ok(
            r#"export const value = 1;
export function add() { return value; }
export default class Thing {}
export { value as renamed };
export { default as other } from "dep";
export * from "all";
export * as namespace from "ns";"#,
        );
        assert!(output.contains("const value = 1;"), "{output}");
        assert!(output.contains("class Thing {}"), "{output}");
        assert!(
            output.contains("Object.defineProperty(exports, \"value\""),
            "{output}"
        );
        assert!(
            output.contains("Object.defineProperty(exports, \"renamed\""),
            "{output}"
        );
        assert!(
            output.contains("Object.defineProperty(exports, \"other\""),
            "{output}"
        );
        assert!(
            output.contains("_tnpm$exportStar(exports, _tnpm$m1);"),
            "{output}"
        );
        assert!(
            output.contains("exports[\"namespace\"] = _tnpm$interopWildcard(_tnpm$m2);"),
            "{output}"
        );
    }

    #[test]
    fn local_exports_are_defined_before_star_exports() {
        let output = lower_ok(r#"export * from "dep"; export const shared = "local";"#);
        let local = output.find("exports, \"shared\"").unwrap();
        let star = output.find("_tnpm$exportStar(exports,").unwrap();
        assert!(local < star, "{output}");
    }

    #[test]
    fn default_function_exports_are_live() {
        let output = lower_ok("export default function run() {}");
        assert!(output.contains("function run() {}"), "{output}");
        assert!(
            output.contains("Object.defineProperty(exports, \"default\""),
            "{output}"
        );
        assert!(output.contains("return run;"), "{output}");
    }

    #[test]
    fn import_references_preserve_live_bindings_and_shorthand() {
        let output = lower_ok(
            r#"import { counter, bump } from "dep";
const snapshot = { counter };
bump();
console.log(counter, snapshot);"#,
        );
        assert!(
            output.contains("const snapshot = { counter: _tnpm$m0.counter };"),
            "{output}"
        );
        assert!(output.contains("(0, _tnpm$m0.bump)();"), "{output}");
        assert!(
            output.contains("console.log(_tnpm$m0.counter, snapshot);"),
            "{output}"
        );
    }

    #[test]
    fn exported_import_bindings_use_import_expressions() {
        let output = lower_ok(
            r#"import { value } from "dep";
export { value };
export const copy = value;
export default value;"#,
        );
        assert!(
            output.contains("get: function () { return _tnpm$m0.value; }"),
            "{output}"
        );
        assert!(output.contains("const copy = _tnpm$m0.value;"), "{output}");
        assert!(
            output.contains("exports.default = _tnpm$m0.value;"),
            "{output}"
        );
    }

    #[test]
    fn avoids_string_names_and_prefix_collisions() {
        let output = lower_ok(
            r#"const _tnpm$m0 = 1;
import { "x-y" as weird } from "dep";
export { weird as "renamed-name" };"#,
        );
        assert!(
            output.contains("const _tnpm$m1 = require(\"dep\");"),
            "{output}"
        );
        assert!(
            output.contains("Object.defineProperty(exports, \"renamed-name\""),
            "{output}"
        );
        assert!(output.contains("_tnpm$m1[\"x-y\"]"), "{output}");
    }
}
