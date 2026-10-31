//! Lower Oxc's JavaScript output to CommonJS for Node's CommonJS loader.
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use swc_core::{
    common::{FileName, GLOBALS, Globals, Mark, SourceMap, sync::Lrc},
    ecma::{
        ast::{Pass, Program},
        codegen::{Emitter, text_writer::JsWriter},
        parser::{Parser, StringInput, Syntax},
        transforms::{
            base::{
                fixer::fixer,
                helpers::{HELPERS, Helpers, inject_helpers},
                hygiene::hygiene,
                resolver,
            },
            module::{
                common_js::{FeatureFlag, common_js},
                import_analysis::import_analyzer,
                path::Resolver,
                util::Config,
            },
        },
    },
};

pub fn transform(path: &Path, source: String) -> Result<String> {
    GLOBALS.set(&Globals::new(), || {
        HELPERS.set(&Helpers::new(false), || {
            let map: Lrc<SourceMap> = Default::default();
            let file = map.new_source_file(FileName::Real(path.to_owned()).into(), source);
            let mut parser = Parser::new(Syntax::default(), StringInput::from(&*file), None);
            let module = parser.parse_module().map_err(|error| {
                anyhow!("could not parse transformed {}: {error:?}", path.display())
            })?;
            if !parser.take_errors().is_empty() {
                return Err(anyhow!("could not parse transformed {}", path.display()));
            }
            let mut program = Program::Module(module);
            let unresolved = Mark::new();
            let top_level = Mark::new();
            let config = Config {
                ignore_dynamic: true,
                ..Default::default()
            };
            resolver(unresolved, top_level, false).process(&mut program);
            import_analyzer(config.import_interop(), config.ignore_dynamic).process(&mut program);
            common_js(
                Resolver::Default,
                unresolved,
                config,
                FeatureFlag::default(),
            )
            .process(&mut program);
            inject_helpers(unresolved).process(&mut program);
            hygiene().process(&mut program);
            fixer(None).process(&mut program);
            let mut output = Vec::new();
            {
                let mut emitter = Emitter {
                    cfg: Default::default(),
                    cm: map.clone(),
                    comments: None,
                    wr: JsWriter::new(map, "\n", &mut output, None),
                };
                emitter
                    .emit_program(&program)
                    .context("could not emit CommonJS")?;
            }
            String::from_utf8(output).context("CommonJS output is not UTF-8")
        })
    })
}
