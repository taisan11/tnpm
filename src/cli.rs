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
    pub legacy_peer_deps: Option<bool>,
    pub strict_peer_deps: bool,
    pub force: bool,
    pub offline: bool,
    pub latest: bool,
    pub plain: bool,
}

const COMMANDS: &[&str] = &[
    "ls",
    "fund",
    "version",
    "upgrade",
    "view",
    "get",
    "cache",
    "install",
    "uninstall",
    "ci",
    "exec",
    "init",
    "create",
    "outdated",
    "update",
    "prune",
    "run",
    "alias",
];

const HELP: &str = "tnpm - A fast, small npm-compatible package manager\n\nUsage: tnpm [OPTIONS] <COMMAND|SCRIPT> [ARGS...]\n\nCommands:\n  ls, fund, version, upgrade, view, get, cache\n  install (alias: i, add)\n  uninstall (alias: remove, rm)\n  exec (alias: x)\n  ci, init\n  create\n  alias create, alias delete\n  outdated, update, prune, run\n\nOptions:\n  --json  --no-cache  --prefix <PATH>  --registry <URL>\n  --userconfig <PATH>  --globalconfig <PATH>  --latest\n  --all, -a  --depth <N>  --preid <ID>  --no-git-tag-version\n  --allow-same-version  --ignore-scripts  --legacy-peer-deps\n  --no-legacy-peer-deps  --strict-peer-deps  --force, -f\n  --offline  --save-dev, -D  --save-optional, -O\n  --save-exact, -E  --no-save  --save, -S\n  --package-lock-only  --omit <TYPES>  --production  --if-present\n  --yes, -y  --call, -c <COMMAND>  --package, -p <PACKAGE>  --plain\n  -h, --help  -V, --version\n";

/// Parse tnpm options until an operand starts a script or executable's arguments.
pub fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Option<Options>> {
    use anyhow::{Context, bail};
    let raw: Vec<String> = args.into_iter().collect();
    let mut options = Options::default();
    let mut i = 0;
    while i < raw.len() {
        let arg = &raw[i];
        if arg == "--" {
            options.passthrough.extend_from_slice(&raw[i + 1..]);
            break;
        }
        if arg.starts_with('-') && arg != "-" {
            let (name, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(n, v)| (n, Some(v)));
            let takes_value = matches!(
                name,
                "--prefix"
                    | "--registry"
                    | "--userconfig"
                    | "--globalconfig"
                    | "--depth"
                    | "--preid"
                    | "--omit"
                    | "--call"
                    | "-c"
                    | "--package"
                    | "-p"
            );
            let value = if takes_value {
                Some(if let Some(value) = inline {
                    value.to_owned()
                } else {
                    i += 1;
                    raw.get(i)
                        .with_context(|| format!("{name} requires a value"))?
                        .clone()
                })
            } else {
                if inline.is_some() {
                    bail!("{name} does not take a value");
                }
                None
            };
            match name {
                "--help" | "-h" => {
                    print!("{HELP}");
                    return Ok(None);
                }
                "--version" | "-V" => {
                    println!("{}", env!("CARGO_PKG_VERSION"));
                    return Ok(None);
                }
                "--json" => options.json = true,
                "--no-cache" => options.no_cache = true,
                "--prefix" => options.prefix = value.map(PathBuf::from),
                "--registry" => options.registry = value,
                "--userconfig" => options.userconfig = value,
                "--globalconfig" => options.globalconfig = value,
                "--all" | "-a" if matches!(options.command.as_str(), "ls" | "prune") => {
                    options.all = true
                }
                "--depth" if options.command == "ls" => {
                    options.depth = Some(value.unwrap().parse().context("invalid --depth")?)
                }
                "--preid" if options.command == "version" => options.preid = value,
                "--no-git-tag-version" if options.command == "version" => options.no_git_tag = true,
                "--allow-same-version" if options.command == "version" => {
                    options.allow_same_version = true
                }
                "--latest" if options.command == "update" => options.latest = true,
                "--plain" if options.command == "outdated" => options.plain = true,
                "--ignore-scripts" => options.ignore_scripts = true,
                "--legacy-peer-deps" => options.legacy_peer_deps = Some(true),
                "--no-legacy-peer-deps" => options.legacy_peer_deps = Some(false),
                "--strict-peer-deps" => options.strict_peer_deps = true,
                "--force" | "-f" => options.force = true,
                "--offline" => options.offline = true,
                "--save-dev" | "-D" => options.save_dev = true,
                "--save-optional" | "-O" => options.save_optional = true,
                "--save-exact" | "-E" => options.save_exact = true,
                "--no-save" => options.no_save = true,
                "--save" | "-S" => {}
                "--package-lock-only" => options.lock_only = true,
                "--omit" => options.omit = value,
                "--production" => options.production = true,
                "--if-present" => options.if_present = true,
                "--yes" | "-y" => options.yes = true,
                "--call" | "-c" => options.call = value,
                "--package" | "-p" => options.exec_packages.push(value.unwrap()),
                _ => bail!("unknown option: {name}"),
            }
        } else if options.command.is_empty() {
            let canonical = COMMAND_ALIASES.get(arg.as_str()).copied().unwrap_or(arg);
            if COMMANDS.contains(&canonical) {
                options.command = canonical.to_owned();
            } else {
                options.command = "run".into();
                options.arguments.push(arg.clone());
                let tail = &raw[i + 1..];
                options
                    .passthrough
                    .extend_from_slice(tail.strip_prefix(&["--".to_owned()]).unwrap_or(tail));
                break;
            }
        } else {
            options.arguments.push(arg.clone());
            if matches!(options.command.as_str(), "run" | "exec" | "create") {
                let tail = &raw[i + 1..];
                options
                    .passthrough
                    .extend_from_slice(tail.strip_prefix(&["--".to_owned()]).unwrap_or(tail));
                break;
            }
        }
        i += 1;
    }
    if options.command.is_empty() {
        if !options.passthrough.is_empty() {
            bail!("missing command");
        }
        print!("{HELP}");
        return Ok(None);
    }
    if options.command == "version" && options.arguments.len() > 1 {
        bail!("version accepts at most one version or release type");
    }
    if options.command == "alias"
        && (options.arguments.len() != 1
            || !matches!(options.arguments[0].as_str(), "create" | "delete")
            || !options.passthrough.is_empty())
    {
        bail!("usage: tnpm alias <create|delete>");
    }
    if options.command == "upgrade"
        && (!options.arguments.is_empty() || !options.passthrough.is_empty())
    {
        bail!("upgrade does not accept arguments");
    }
    if options.command == "init" {
        options.arguments.append(&mut options.passthrough);
    }
    if options.command == "init" && options.arguments.len() > 1 {
        bail!("init accepts at most one path; use create NAME to run an initializer");
    }
    if options.command == "create" && options.arguments.is_empty() {
        bail!("create requires an initializer name");
    }
    if options.command == "fund" && !options.arguments.is_empty() {
        bail!("fund currently lists all funding URLs; package/browser selection is not supported");
    }
    Ok(Some(options))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(args: &[&str]) -> Options {
        parse(args.iter().map(|s| (*s).to_owned()))
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
