use crate::{cli::Options, json::Value};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
};

mod credentials;
mod http;

static PATH_KEYS: phf::Set<&'static str> = phf::phf_set! {
    "cache", "prefix", "userconfig", "globalconfig", "store-dir", "cafile", "certfile", "keyfile",
};

static PROTECTED_FIELDS: phf::Set<&'static str> = phf::phf_set! {
    "username", "keyfile", "cert", "key",
};

pub fn project_root(start: &Path) -> PathBuf {
    let start = if start.is_absolute() {
        start.to_owned()
    } else {
        env::current_dir().unwrap_or_default().join(start)
    };
    start
        .ancestors()
        .find(|path| path.join("package.json").is_file())
        .unwrap_or(&start)
        .to_owned()
}

pub struct Config {
    values: BTreeMap<String, String>,
    arrays: BTreeMap<String, Vec<String>>,
}

impl Config {
    pub fn load(root: &Path, options: &Options) -> Result<Self> {
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_default();
        let env_values: BTreeMap<_, _> = env::vars()
            .filter_map(|(key, value)| {
                key.get(..11)
                    .filter(|prefix| prefix.eq_ignore_ascii_case("npm_config_"))
                    .map(|_| (env_key(&key[11..]), value))
            })
            .collect();
        let mut prefix = env_values.get("prefix").cloned().unwrap_or_else(|| {
            if options.global {
                format!("{home}/.tnpm/global")
            } else {
                "/usr/local".into()
            }
        });
        let userconfig = expand_path(
            &options
                .userconfig
                .clone()
                .or_else(|| env_values.get("userconfig").cloned())
                .unwrap_or_else(|| format!("{home}/.npmrc")),
            &home,
        );
        // User configuration can select the prefix/global config file too.
        let mut user = Self {
            values: BTreeMap::new(),
            arrays: BTreeMap::new(),
        };
        user.read_npmrc(&PathBuf::from(&userconfig))?;
        if !env_values.contains_key("prefix")
            && let Some(value) = user.get("prefix")
        {
            prefix = value.to_owned();
        }
        let globalconfig = expand_path(
            &options
                .globalconfig
                .clone()
                .or_else(|| env_values.get("globalconfig").cloned())
                .or_else(|| user.get("globalconfig").map(str::to_owned))
                .unwrap_or_else(|| format!("{prefix}/etc/npmrc")),
            &home,
        );
        let mut config = Self {
            values: BTreeMap::from([
                ("registry".into(), "https://registry.npmjs.org/".into()),
                ("@jsr:registry".into(), "https://npm.jsr.io/".into()),
                ("tag".into(), "latest".into()),
                ("prefix".into(), prefix),
                ("cache".into(), format!("{home}/.npm")),
                ("store-dir".into(), format!("{home}/.tnpm/cache")),
                ("node-linker".into(), "hoisted".into()),
                ("toolchain-tsrs".into(), "auto".into()),
                ("userconfig".into(), userconfig.clone()),
                ("globalconfig".into(), globalconfig.clone()),
                ("git-tag-version".into(), "true".into()),
                ("tag-version-prefix".into(), "v".into()),
                ("ignore-scripts".into(), "false".into()),
                ("allow-same-version".into(), "false".into()),
                ("preid".into(), String::new()),
            ]),
            arrays: BTreeMap::new(),
        };
        for path in [
            PathBuf::from(globalconfig),
            PathBuf::from(userconfig),
            root.join(".npmrc"),
        ] {
            config.read_npmrc(&path)?;
        }
        for (key, value) in env_values {
            let value = if path_key(&key) {
                expand_path(&value, &home)
            } else {
                value
            };
            config.set(key, value);
        }
        if let Some(path) = &options.userconfig {
            config.set("userconfig".into(), expand_path(path, &home));
        }
        if let Some(path) = &options.globalconfig {
            config.set("globalconfig".into(), expand_path(path, &home));
        }
        if let Some(linker) = &options.node_linker {
            config.set("node-linker".into(), linker.clone());
        }
        if let Some(store_dir) = &options.store_dir {
            config.set("store-dir".into(), store_dir.display().to_string());
        }
        match config.get("node-linker") {
            Some("hoisted" | "isolated") => {}
            _ => bail!("node-linker must be hoisted or isolated"),
        }
        if !config
            .get("store-dir")
            .is_some_and(|value| Path::new(value).is_absolute())
        {
            bail!("store-dir must be an absolute path");
        }
        if !matches!(
            config.get("toolchain-tsrs"),
            Some("auto" | "on" | "off" | "true" | "false")
        ) {
            bail!("toolchain-tsrs must be auto, on, or off");
        }
        if let Some(registry) = &options.registry {
            config.set("registry".into(), registry.clone());
        }
        if let Some(prefix) = &options.prefix {
            config
                .values
                .insert("prefix".into(), prefix.display().to_string());
        }
        for key in [
            "_auth",
            "_authToken",
            "username",
            "_password",
            "certfile",
            "keyfile",
        ] {
            if config.get(key).is_some_and(|v| !v.is_empty()) {
                bail!("{key} must be scoped to a registry in .npmrc (//host/path/:{key})");
            }
        }
        Ok(config)
    }

    fn set(&mut self, key: String, value: String) {
        self.arrays.remove(&key);
        self.values.insert(key, value);
    }

    fn read_npmrc(&mut self, path: &Path) -> Result<()> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
        };
        let mut arrays = BTreeSet::new();
        for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let (key, value) = line.split_once('=').unwrap_or((line, "true"));
            let key = key.trim();
            // npm consumes flat ini keys; section headers have no config meaning.
            if key.is_empty() || (key.starts_with('[') && key.ends_with(']')) {
                continue;
            }
            let value = expand_env(&ini_value(value));
            let value = if path_key(key.trim_end_matches("[]")) {
                expand_path(
                    &value,
                    &env::var("HOME")
                        .or_else(|_| env::var("USERPROFILE"))
                        .unwrap_or_default(),
                )
            } else {
                value
            };
            if let Some(key) = key.strip_suffix("[]") {
                if arrays.insert(key.to_owned()) {
                    self.arrays.insert(key.to_owned(), Vec::new());
                }
                let items = self.arrays.get_mut(key).expect("array initialized");
                items.push(value);
                self.values.insert(key.to_owned(), items.join(","));
            } else {
                self.set(key.into(), value);
                arrays.remove(key);
            }
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn enabled(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }

    pub fn print(&self, keys: &[String], json: bool) -> Result<()> {
        for key in keys {
            if protected(key) {
                bail!("{key} is protected and cannot be retrieved");
            }
        }
        if json || keys.is_empty() {
            let mut result = Value::object();
            for (key, value) in &self.values {
                if !protected(key) && (keys.is_empty() || keys.contains(key)) {
                    result.set(
                        key,
                        if let Some(items) = self.arrays.get(key) {
                            Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect())
                        } else {
                            match value.as_str() {
                                "true" => Value::Bool(true),
                                "false" => Value::Bool(false),
                                _ => Value::from(value.as_str()),
                            }
                        },
                    );
                }
            }
            if json && keys.len() == 1 {
                println!("{}", result.get(&keys[0]).unwrap_or(&Value::Null).pretty());
            } else if json {
                println!("{}", result.pretty());
            } else {
                for (key, value) in result.members() {
                    println!("{key}={}", value.display());
                }
            }
        } else {
            for key in keys {
                println!("{}", self.get(key).unwrap_or("undefined"));
            }
        }
        Ok(())
    }
}

fn protected(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("auth")
        || key.contains("password")
        || key.contains("token")
        || PROTECTED_FIELDS.contains(key.rsplit(':').next().unwrap_or(&key))
}

fn expand_env(value: &str) -> String {
    let mut result = String::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start + 2..].find('}') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let name = &rest[start + 2..start + 2 + end];
        let (name, empty) = name
            .strip_suffix('?')
            .map(|n| (n, true))
            .unwrap_or((name, false));
        result.push_str(&env::var(name).unwrap_or_else(|_| {
            if empty {
                String::new()
            } else {
                format!("${{{name}}}")
            }
        }));
        rest = &rest[start + 3 + end..];
    }
    result.push_str(rest);
    result
}

fn env_key(key: &str) -> String {
    let lower = key.to_ascii_lowercase();
    if lower == "_authtoken" {
        "_authToken".into()
    } else if key.starts_with('_') {
        lower
    } else if key.starts_with(['/', '@']) {
        key.into()
    } else {
        lower.replace('_', "-")
    }
}

fn path_key(key: &str) -> bool {
    PATH_KEYS.contains(key.rsplit(':').next().unwrap_or(key))
}

fn expand_path(value: &str, home: &str) -> String {
    value
        .strip_prefix("~/")
        .map(|rest| format!("{home}/{rest}"))
        .unwrap_or_else(|| value.into())
}

fn ini_value(value: &str) -> String {
    let value = value.trim();
    if let Some(quote @ ('\'' | '"')) = value.chars().next() {
        let mut escaped = false;
        for (index, c) in value.char_indices().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' && quote == '"' {
                escaped = true;
                continue;
            }
            if c == quote {
                let tail = value[index + 1..].trim_start();
                if tail.is_empty() || tail.starts_with(['#', ';']) {
                    if quote == '"'
                        && let Ok(decoded) = Value::parse(&value[..=index])
                        && let Some(decoded) = decoded.string()
                    {
                        return decoded.into();
                    }
                    return value[1..index].into();
                }
                break;
            }
        }
    }
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(c, ';' | '#') {
            break;
        }
        if c == '\\' && chars.peek().is_some_and(|c| matches!(c, ';' | '#' | '\\')) {
            result.push(chars.next().unwrap());
        } else {
            result.push(c);
        }
    }
    result.trim().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protects_registry_tokens() {
        assert!(protected("//registry.npmjs.org/:_authToken"));
        assert!(protected("_password"));
        assert!(protected("//registry.test/:username"));
        assert!(protected("//registry.test/:keyfile"));
        assert!(!protected("registry"));
    }

    #[test]
    fn fixed_keys_match_only_whole_fields_and_preserve_scoped_behavior() {
        for key in [
            "cache",
            "prefix",
            "userconfig",
            "globalconfig",
            "cafile",
            "certfile",
            "keyfile",
        ] {
            assert!(path_key(key));
            assert!(path_key(&format!("//registry.test:8443/team/:{key}")));
        }
        for key in [
            "registry",
            "node-options",
            "cache-extra",
            "mykeyfile",
            "KEYFILE",
        ] {
            assert!(!path_key(key), "{key}");
        }
        for key in ["username", "keyfile", "cert", "key"] {
            assert!(protected(key));
            assert!(protected(&format!("//registry.test:8443/team/:{key}")));
        }
        assert!(protected("custom-auth-token"));
        assert!(!protected("monkeyfile"));
        assert!(!protected("certfile"));
    }

    #[test]
    fn environment_names_preserve_auth_keys_and_normalize_options() {
        assert_eq!(env_key("STRICT_PEER_DEPS"), "strict-peer-deps");
        assert_eq!(env_key("_AUTHTOKEN"), "_authToken");
        assert_eq!(env_key("_PASSWORD"), "_password");
        assert_eq!(
            env_key("//registry.test/Team/:_authToken"),
            "//registry.test/Team/:_authToken"
        );
    }

    #[test]
    fn ini_values_preserve_quoted_comments_and_unescape_plain_values() {
        assert_eq!(ini_value("'a#b;c' ; comment"), "a#b;c");
        assert_eq!(ini_value(r##""a\"#b;c" # comment"##), "a\"#b;c");
        assert_eq!(ini_value(r"a\#b\;c # comment"), "a#b;c");
        assert_eq!(ini_value("  value ; comment"), "value");
    }
}
