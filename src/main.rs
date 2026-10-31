mod app;
mod cli;
mod commands;
mod config;
mod fs;
mod install;
mod json;
mod lockfile;
mod manifest;
mod profile;
mod progress;
mod registry;
mod runner;
mod scripts;
mod semver;
mod spec;
mod style;
mod table;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> std::process::ExitCode {
    if let Some(result) = runner::internal() {
        return match result {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }
    style::init();
    let result = app::run();
    progress::finish();
    let status = match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                style::stderr(&format!("tnpm: {error}"), style::Tone::Error)
            );
            for cause in error.chain().skip(1) {
                eprintln!(
                    "{}",
                    style::stderr(&format!("  caused by: {cause}"), style::Tone::Muted)
                );
            }
            if let Some(error) = error.downcast_ref::<scripts::CommandFailed>() {
                std::process::ExitCode::from(error.0.code().unwrap_or(1).clamp(1, 255) as u8)
            } else {
                std::process::ExitCode::FAILURE
            }
        }
    };
    // Flush machine-readable/normal output before the final diagnostic footer.
    use std::io::Write;
    let _ = std::io::stdout().flush();
    eprintln!(
        "{}",
        style::stderr(
            &format!(
                "tnpm v{}({})",
                env!("CARGO_PKG_VERSION"),
                env!("TNPM_BUILD_COMMIT")
            ),
            style::Tone::Muted
        )
    );
    status
}
