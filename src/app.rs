//! Application setup and command dispatch; process exit handling stays in main.
use crate::{cli, commands, config, install};
use anyhow::{Context, Result};

pub fn run() -> Result<()> {
    let Some(options) = cli::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    // Cache maintenance is global and must work without a project or a valid npmrc.
    if options.command == "cache" {
        return commands::cache::run(&options);
    }
    if options.command == "alias" {
        return commands::alias::run(&options);
    }
    if options.command == "upgrade" {
        return tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(commands::upgrade::run(&options));
    }
    let cwd = std::env::current_dir().context("could not determine current directory")?;
    let start = options.prefix.as_deref().unwrap_or(&cwd);
    let target = if options.command == "init" {
        options.arguments.first().map(|path| start.join(path))
    } else {
        None
    };
    let start = target.as_deref().unwrap_or(start);
    let root = if options.prefix.is_some() || matches!(options.command.as_str(), "init" | "create")
    {
        if start.is_absolute() {
            start.to_owned()
        } else {
            cwd.join(start)
        }
    } else {
        config::project_root(start)
    };
    let config = config::Config::load(&root, &options)?;
    match options.command.as_str() {
        "ls" => commands::packages::list(&root, &options),
        "fund" => commands::packages::fund(&root, &options),
        "version" => commands::version::run(&root, &options, &config),
        "get" => config.print(&options.arguments, options.json),
        "run" => commands::run::run(&root, &options, &config),
        "init" => commands::init::run(&root, &options),
        "view" | "install" | "uninstall" | "ci" | "exec" | "create" | "outdated" | "update"
        | "prune" => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                if options.command == "view" {
                    commands::view::run(&root, &options, &config).await
                } else {
                    install::run(&root, &options, &config).await
                }
            }),
        _ => unreachable!(),
    }
}
