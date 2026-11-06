//! Application setup and command dispatch; process exit handling stays in main.
use crate::{cli, commands, config, install};
use anyhow::{Context, Result};

pub fn run() -> Result<()> {
    let Some(mut options) = cli::parse(std::env::args().skip(1))? else {
        return Ok(());
    };
    if options.global {
        return crate::global::run(&options);
    }
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
    let mut root =
        if options.prefix.is_some() || matches!(options.command.as_str(), "init" | "create") {
            if start.is_absolute() {
                start.to_owned()
            } else {
                cwd.join(start)
            }
        } else {
            config::project_root(start)
        };
    if !matches!(options.command.as_str(), "init" | "create" | "exec") {
        if let Some(workspace_root) = crate::workspaces::root(&root)? {
            if (options.command == "run"
                || options.command == "uninstall"
                || (options.command == "install" && !options.arguments.is_empty())
                || (options.command == "update" && options.latest))
                && options.workspace.is_empty()
                && !options.workspaces
            {
                options.workspace.push(
                    root.strip_prefix(&workspace_root)?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            root = workspace_root;
        }
        if options.workspaces || !options.workspace.is_empty() {
            let data = crate::manifest::read(&root)?.context("workspaces require package.json")?;
            let workspaces = crate::workspaces::discover(&root, &data)?;
            let selected = crate::workspaces::selected(&workspaces, &options.workspace)?;
            anyhow::ensure!(!selected.is_empty(), "no workspaces found");
            if options.command == "run" {
                let config = config::Config::load(&root, &options)?;
                for workspace in selected {
                    commands::run::run(&root.join(&workspace.relative), &options, &config)?;
                }
                return Ok(());
            }
            // Install is a single shared graph. Manifest mutation is scoped separately.
            anyhow::ensure!(
                matches!(
                    options.command.as_str(),
                    "install" | "ci" | "update" | "uninstall"
                ),
                "workspace selection supports run, install, ci, update and uninstall"
            );
        }
    }
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
