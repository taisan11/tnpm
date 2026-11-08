use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_resolver::{ResolveOptions, Resolver, TsconfigDiscovery};
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{TransformOptions, Transformer};

/// Transform a JavaScript or TypeScript file into executable JavaScript.
pub fn transform(path: &Path, commonjs: bool) -> Result<String> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    transform_source(path, source, commonjs)
}

pub(super) fn transform_source(path: &Path, source: String, commonjs: bool) -> Result<String> {
    let source_type = SourceType::from_path(path)
        .map_err(|err| anyhow!("unsupported source file {}: {err}", path.display()))?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, &source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        let diagnostics = parsed
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic
                    .render_with_source_code(source.clone())
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!("failed to parse {}:\n{diagnostics}", path.display());
    }

    let mut program = parsed.program;
    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(&program);
    if !semantic.diagnostics.is_empty() {
        let diagnostics = semantic
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic
                    .render_with_source_code(source.clone())
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!("failed to analyze {}:\n{diagnostics}", path.display());
    }

    let mut options = TransformOptions::default();
    if commonjs {
        options.env.module = oxc_transformer::Module::CommonJS;
    }
    let transformed = Transformer::new(&allocator, path, &options)
        .build_with_scoping(semantic.semantic.into_scoping(), &mut program);
    if !transformed.diagnostics.is_empty() {
        let diagnostics = transformed
            .diagnostics
            .into_iter()
            .map(|diagnostic| {
                diagnostic
                    .render_with_source_code(source.clone())
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        bail!("failed to transform {}:\n{diagnostics}", path.display());
    }

    let code = Codegen::new().build(&program).code;
    if commonjs {
        super::commonjs::transform(path, code)
    } else {
        Ok(code)
    }
}

/// Resolve an import from the importing file, using its nearest tsconfig.
pub fn resolve(parent: &Path, specifier: &str, conditions: &[String]) -> Result<PathBuf> {
    resolve_with(parent, specifier, conditions, true)
}

/// Protocol imports identify a package, never a tsconfig alias with the same name.
pub(super) fn resolve_package(
    directory: &Path,
    specifier: &str,
    conditions: &[String],
) -> Result<PathBuf> {
    resolve_with(
        &directory.join(".tnpm-runner-import.mjs"),
        specifier,
        conditions,
        false,
    )
}

fn resolve_with(
    parent: &Path,
    specifier: &str,
    conditions: &[String],
    tsconfig: bool,
) -> Result<PathBuf> {
    let parent = if parent.is_absolute() {
        parent.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to determine current directory")?
            .join(parent)
    };
    let options = ResolveOptions {
        tsconfig: tsconfig.then_some(TsconfigDiscovery::Auto),
        condition_names: conditions.to_vec(),
        extensions: [
            ".js", ".json", ".node", ".ts", ".tsx", ".jsx", ".mts", ".cts", ".mjs", ".cjs",
        ]
        .into_iter()
        .map(String::from)
        .collect(),
        extension_alias: vec![
            (
                ".js".into(),
                vec![".js".into(), ".ts".into(), ".tsx".into(), ".jsx".into()],
            ),
            (".mjs".into(), vec![".mjs".into(), ".mts".into()]),
            (".cjs".into(), vec![".cjs".into(), ".cts".into()]),
        ],
        ..ResolveOptions::default()
    };
    let resolver = Resolver::new(options);
    let resolution = resolver
        .resolve_file(&parent, specifier)
        .or_else(|error| {
            // TypeScript excludes JS from config discovery without allowJs, but
            // JS importers should still be able to use configured path aliases.
            if matches!(
                parent.extension().and_then(|s| s.to_str()),
                Some("js" | "jsx" | "mjs" | "cjs")
            ) {
                resolver.resolve_file(parent.with_extension("ts"), specifier)
            } else {
                Err(error)
            }
        })
        .with_context(|| format!("failed to resolve {specifier:?} from {}", parent.display()))?;
    Ok(resolution.into_path_buf())
}
