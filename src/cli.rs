use std::path::PathBuf;

static COMMAND_ALIASES: phf::Map<&'static str, &'static str> = phf::phf_map! {
    "i" => "install",
    "add" => "install",
    "remove" => "uninstall",
    "rm" => "uninstall",
    "x" => "exec",
};

#[derive(Debug, Default)]
pub struct Options {
    pub command: String,
    pub arguments: Vec<String>,
    pub json: bool,
    pub no_cache: bool,
    pub all: bool,
    pub depth: Option<usize>,
    pub prefix: Option<PathBuf>,
    pub registry: Option<String>,
    pub userconfig: Option<String>,
    pub globalconfig: Option<String>,
    pub preid: Option<String>,
    pub no_git_tag: bool,
    pub ignore_scripts: bool,
    pub allow_same_version: bool,
    pub save_dev: bool,
    pub save_optional: bool,
    pub save_exact: bool,
    pub no_save: bool,
    pub lock_only: bool,
    pub omit: Option<String>,
    pub production: bool,
    pub if_present: bool,
    pub yes: bool,
    pub exec_packages: Vec<String>,
    pub call: Option<String>,
    pub passthrough: Vec<String>,
    pub legacy_peer_deps: bool,
    pub strict_peer_deps: bool,
    pub force: bool,
    pub offline: bool,
    pub latest: bool,
    pub plain: bool,
}

pub fn parse(mut args: noargs::RawArgs) -> noargs::Result<Option<Options>> {
    // Stop interpreting flags at the script/binary/initializer operand or an explicit `--`.
    let raw: Vec<String> = args.remaining_args().map(|(_, s)| s.to_owned()).collect();
    let (head, tail) = split_passthrough(&raw);
    args = noargs::RawArgs::new(std::iter::once("tnpm".into()).chain(head));
    args.metadata_mut().app_name = "tnpm";
    args.metadata_mut().app_description = "A fast, small npm-compatible package manager";
    if noargs::VERSION_FLAG.take(&mut args).is_present() {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(None);
    }
    let mut options = Options {
        json: noargs::flag("json")
            .doc("Output JSON")
            .take(&mut args)
            .is_present(),
        no_cache: noargs::flag("no-cache")
            .doc("Bypass registry cache reads and writes (network commands)")
            .take(&mut args)
            .is_present(),
        prefix: noargs::opt("prefix")
            .doc("Project directory")
            .take(&mut args)
            .present_and_then(|o| o.value().parse())?,
        registry: string_option(&mut args, "registry")?,
        userconfig: string_option(&mut args, "userconfig")?,
        globalconfig: string_option(&mut args, "globalconfig")?,
        passthrough: tail,
        ..Options::default()
    };
    noargs::HELP_FLAG.take_help(&mut args);
    // The help label can include aliases; only the first word is a command name.
    let commands: &[(&str, &str)] = &[
        ("ls", "List installed dependencies"),
        (
            "fund",
            "List dependency funding URLs (does not open a browser)",
        ),
        ("version", "Print versions or bump the package version"),
        (
            "upgrade",
            "Upgrade tnpm itself to the latest GitHub release",
        ),
        ("view", "Read package metadata from a registry"),
        ("get", "Read npm configuration"),
        (
            "cache",
            "Show cache disk usage; cache rm removes all tnpm caches",
        ),
        ("install (alias: i, add)", "Install dependencies"),
        ("uninstall (alias: remove, rm)", "Remove dependencies"),
        ("ci", "Clean install from a matching lockfile"),
        ("exec (alias: x)", "Run a package binary"),
        (
            "init",
            "Create package.json in [PATH] (default: current directory)",
        ),
        ("create", "Run a create-* initializer"),
        ("outdated", "Show outdated direct dependencies"),
        (
            "update",
            "Update dependencies (use --latest to upgrade declared ranges)",
        ),
        ("prune", "Remove extraneous packages"),
        ("run", "Run a package script"),
    ];
    let alias = args
        .remaining_args()
        .next()
        .and_then(|(_, arg)| COMMAND_ALIASES.get_entry(arg))
        .map(|(&alias, &command)| (alias, command));
    for &(label, doc) in commands {
        let command = label.split_whitespace().next().unwrap();
        // Register the actual spelling when matched, otherwise the help label.
        let name = match alias {
            Some((alias, canonical)) if canonical == command => alias,
            _ if args
                .remaining_args()
                .next()
                .is_some_and(|(_, arg)| arg == command) =>
            {
                command
            }
            _ => label,
        };
        if noargs::cmd(name).doc(doc).take(&mut args).is_present() {
            options.command = command.into();
            break;
        }
    }
    if options.command == "update" {
        options.latest = noargs::flag("latest")
            .doc("Upgrade declared dependencies to the latest tag, including major versions")
            .take(&mut args)
            .is_present();
    }
    if options.command == "outdated" {
        options.plain = noargs::flag("plain")
            .doc("Print a borderless, uncolored table")
            .take(&mut args)
            .is_present();
    }
    if options.command == "prune" {
        options.all = noargs::flag("all")
            .short('a')
            .doc("Remove the entire node_modules directory")
            .take(&mut args)
            .is_present();
    }
    match options.command.as_str() {
        "ls" => {
            options.all = noargs::flag("all")
                .short('a')
                .doc("Include transitive dependencies")
                .take(&mut args)
                .is_present();
            options.depth = noargs::opt("depth")
                .doc("Maximum dependency depth (default: 0)")
                .take(&mut args)
                .present_and_then(|o| o.value().parse())?;
        }
        "version" => {
            options.preid = string_option(&mut args, "preid")?;
            options.no_git_tag = noargs::flag("no-git-tag-version")
                .doc("Do not commit or tag")
                .take(&mut args)
                .is_present();
            options.ignore_scripts = noargs::flag("ignore-scripts").take(&mut args).is_present();
            options.allow_same_version = noargs::flag("allow-same-version")
                .take(&mut args)
                .is_present();
        }
        "install" | "uninstall" | "ci" | "update" | "prune" | "exec" | "init" | "create"
        | "run" | "outdated" => {
            options.ignore_scripts = noargs::flag("ignore-scripts").take(&mut args).is_present();
            options.legacy_peer_deps = noargs::flag("legacy-peer-deps")
                .take(&mut args)
                .is_present();
            options.strict_peer_deps = noargs::flag("strict-peer-deps")
                .doc("Fail on peer version mismatches (default: warn and continue)")
                .take(&mut args)
                .is_present();
            options.force = noargs::flag("force")
                .short('f')
                .doc("Allow peer version mismatches even in strict mode")
                .take(&mut args)
                .is_present();
            options.offline = noargs::flag("offline").take(&mut args).is_present();
            options.save_dev = noargs::flag("save-dev")
                .short('D')
                .take(&mut args)
                .is_present();
            options.save_optional = noargs::flag("save-optional")
                .short('O')
                .take(&mut args)
                .is_present();
            options.save_exact = noargs::flag("save-exact")
                .short('E')
                .take(&mut args)
                .is_present();
            options.no_save = noargs::flag("no-save").take(&mut args).is_present();
            noargs::flag("save").short('S').take(&mut args);
            options.lock_only = noargs::flag("package-lock-only")
                .take(&mut args)
                .is_present();
            options.omit = string_option(&mut args, "omit")?;
            options.production = noargs::flag("production").take(&mut args).is_present();
            options.if_present = noargs::flag("if-present").take(&mut args).is_present();
            options.yes = noargs::flag("yes").short('y').take(&mut args).is_present();
            options.call = noargs::opt("call")
                .short('c')
                .take(&mut args)
                .present_and_then(|o| o.value().parse())?;
            while let Some(package) = noargs::opt("package")
                .short('p')
                .take(&mut args)
                .present_and_then(|o| o.value().parse::<String>())?
            {
                options.exec_packages.push(package);
            }
        }
        _ => {}
    }
    if options.command.is_empty() && args.remaining_args().next().is_none() {
        args.metadata_mut().help_mode = true;
    } else if !options.command.is_empty() {
        while let Some(arg) = noargs::arg("[ARG]...")
            .take(&mut args)
            .present_and_then(|a| a.value().parse::<String>())?
        {
            options.arguments.push(arg);
        }
    }
    if let Some(help) = args.finish()? {
        print!("{help}");
        return Ok(None);
    }
    if options.command == "version" && options.arguments.len() > 1 {
        return Err("version accepts at most one version or release type".into());
    }
    if options.command == "upgrade"
        && (!options.arguments.is_empty() || !options.passthrough.is_empty())
    {
        return Err("upgrade does not accept arguments".into());
    }
    if options.command == "init" {
        options.arguments.append(&mut options.passthrough);
    }
    if options.command == "init" && (options.arguments.len() > 1 || !options.passthrough.is_empty())
    {
        return Err("init accepts at most one path; use create NAME to run an initializer".into());
    }
    if options.command == "create" && options.arguments.is_empty() {
        return Err("create requires an initializer name".into());
    }
    if options.command == "fund" && !options.arguments.is_empty() {
        return Err(
            "fund currently lists all funding URLs; package/browser selection is not supported"
                .into(),
        );
    }
    Ok(Some(options))
}

fn split_passthrough(raw: &[String]) -> (Vec<String>, Vec<String>) {
    let mut command = None;
    let mut skip = false;
    for (i, arg) in raw.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--" {
            return (raw[..i].to_vec(), raw[i + 1..].to_vec());
        }
        if matches!(
            arg.as_str(),
            "--prefix"
                | "--registry"
                | "--userconfig"
                | "--globalconfig"
                | "--package"
                | "-p"
                | "--call"
                | "-c"
                | "--omit"
        ) {
            skip = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        if command.is_none() {
            command = Some(arg.as_str());
        } else if matches!(command, Some("run" | "exec" | "x" | "create")) {
            let tail = &raw[i + 1..];
            return (
                raw[..=i].to_vec(),
                tail.strip_prefix(&["--".to_owned()])
                    .unwrap_or(tail)
                    .to_vec(),
            );
        }
    }
    (raw.to_vec(), Vec::new())
}

fn string_option(args: &mut noargs::RawArgs, name: &'static str) -> noargs::Result<Option<String>> {
    noargs::opt(name)
        .take(args)
        .present_and_then(|o| o.value().parse())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Options {
        parse(noargs::RawArgs::new(
            std::iter::once("tnpm")
                .chain(args.iter().copied())
                .map(str::to_owned),
        ))
        .unwrap()
        .unwrap()
    }
    #[test]
    fn script_and_binary_flags_are_not_cli_flags() {
        let o = options(&["run", "test", "--json", "--version", "--help"]);
        assert!(!o.json);
        assert_eq!(o.passthrough, ["--json", "--version", "--help"]);
        let o = options(&[
            "--prefix",
            "run",
            "x",
            "--package",
            "foo",
            "--",
            "foo",
            "--help",
        ]);
        assert_eq!(o.command, "exec");
        assert_eq!(o.exec_packages, ["foo"]);
        assert_eq!(o.passthrough, ["foo", "--help"]);
    }

    #[test]
    fn aliases_select_canonical_commands() {
        for (alias, command) in [
            ("i", "install"),
            ("add", "install"),
            ("remove", "uninstall"),
            ("rm", "uninstall"),
            ("x", "exec"),
        ] {
            assert_eq!(options(&[alias]).command, command);
        }
        let o = options(&["init", "-y"]);
        assert_eq!(o.command, "init");
        assert!(o.yes);
    }

    #[test]
    fn alias_lookup_is_exact_and_case_sensitive() {
        for name in ["I", "Create", "rm-extra", "install", "unknown", ""] {
            assert!(COMMAND_ALIASES.get(name).is_none(), "{name}");
        }
        assert_eq!(options(&["install"]).command, "install");
    }

    #[test]
    fn cache_rm_is_an_operand_not_the_uninstall_alias() {
        let o = options(&["cache", "rm", "--json"]);
        assert_eq!(o.command, "cache");
        assert_eq!(o.arguments, ["rm"]);
        assert!(o.json);
        assert!(options(&["cache"]).arguments.is_empty());
    }

    #[test]
    fn update_latest_is_a_flag_not_a_package_filter() {
        let o = options(&["update", "--latest"]);
        assert!(o.latest);
        assert!(o.arguments.is_empty());
        let o = options(&["update", "@scope/package", "--latest", "--force"]);
        assert!(o.latest);
        assert!(o.force);
        assert_eq!(o.arguments, ["@scope/package"]);
    }

    #[test]
    fn initializer_arguments_are_forwarded_unchanged() {
        {
            let command = "create";
            let o = options(&[
                "--prefix",
                "project",
                command,
                "--no-cache",
                "vite@latest",
                "my-app",
                "--template",
                "react",
                "--json",
                "--help",
                "--version",
            ]);
            assert_eq!(o.command, "create");
            assert_eq!(o.prefix, Some(PathBuf::from("project")));
            assert!(o.no_cache);
            assert!(!o.json);
            assert_eq!(o.arguments, ["vite@latest"]);
            assert_eq!(
                o.passthrough,
                [
                    "my-app",
                    "--template",
                    "react",
                    "--json",
                    "--help",
                    "--version"
                ]
            );

            let o = options(&[command, "vite", "--", "my-app", "--template", "react"]);
            assert_eq!(o.arguments, ["vite"]);
            assert_eq!(o.passthrough, ["my-app", "--template", "react"]);
        }
    }

    #[test]
    fn init_operand_is_a_path_and_flags_are_not_forwarded() {
        let o = options(&["init", "vite", "--json", "-y"]);
        assert_eq!(o.command, "init");
        assert_eq!(o.arguments, ["vite"]);
        assert!(o.json && o.yes);
        assert!(o.passthrough.is_empty());
        let o = options(&["outdated", "--plain"]);
        assert!(o.plain);
    }
}
