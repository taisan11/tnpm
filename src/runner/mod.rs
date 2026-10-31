mod commonjs;
mod oxc;

use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    env,
    io::{self, Write},
    path::Path,
    process::Command,
};

pub fn internal() -> Option<Result<()>> {
    let mut args = env::args_os().skip(1);
    let command = args.next()?;
    if command == "--tnpm-runner-transform" {
        return Some((|| {
            let path = args.next().context("missing transform path")?;
            let format = args.next().context("missing transform format")?;
            let code = oxc::transform(Path::new(&path), format == "commonjs")?;
            io::stdout().write_all(code.as_bytes())?;
            Ok(())
        })());
    }
    if command == "--tnpm-runner-resolve" {
        return Some((|| {
            let parent = args.next().context("missing parent path")?;
            let specifier = args
                .next()
                .context("missing specifier")?
                .into_string()
                .map_err(|_| anyhow::anyhow!("invalid specifier"))?;
            let conditions = args
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let path = oxc::resolve(Path::new(&parent), &specifier, &conditions)?;
            let path = path.to_str().context("resolved path is not UTF-8")?;
            io::stdout().write_all(path.as_bytes())?;
            Ok(())
        })());
    }
    None
}

pub fn run(cwd: &Path, entry: &Path, args: &[String]) -> Result<()> {
    if !entry.is_file() {
        bail!("runner entry not found: {}", entry.display());
    }
    let source = STANDARD.encode(include_bytes!("loader.mjs"));
    let status = Command::new("node")
        .current_dir(cwd)
        .env("TNPM_RUNNER_EXECUTABLE", env::current_exe()?)
        .arg("--enable-source-maps")
        .arg("--import")
        .arg(format!("data:text/javascript;base64,{source}"))
        .arg(entry)
        .args(args)
        .status()
        .context("could not start Node.js (required by the runner)")?;
    crate::scripts::check(status).context("runner failed")
}
