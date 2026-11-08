//! Deterministic archives: allowlists/ignore files, no links, no local credentials.
use crate::{config::Config, json::Value, scripts};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use glob::{MatchOptions, Pattern};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use sha2::{Digest, Sha512};
use std::{
    fs,
    path::{Component, Path},
};

pub(super) struct File {
    pub path: String,
    pub size: u64,
    mode: u32,
}

pub(super) struct Packed {
    pub id: String,
    pub filename: String,
    pub bytes: Vec<u8>,
    pub integrity: String,
    pub shasum: String,
    pub files: Vec<File>,
}

struct AllowPattern {
    include: bool,
    matcher: Pattern,
}

fn allowlisted(path: &str, patterns: &[AllowPattern]) -> bool {
    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    let mut included = false;
    for pattern in patterns {
        // A directory included by a files entry includes all of its descendants.
        if Path::new(path)
            .ancestors()
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .any(|ancestor| pattern.matcher.matches_path_with(ancestor, options))
        {
            included = pattern.include;
        }
    }
    included
}

impl Packed {
    pub fn summary(&self) -> Value {
        let mut data = Value::object();
        data.set("id", Value::from(self.id.as_str()));
        data.set("filename", Value::from(self.filename.as_str()));
        data.set("size", Value::Number(self.bytes.len().to_string()));
        data.set(
            "unpackedSize",
            Value::Number(self.files.iter().map(|f| f.size).sum::<u64>().to_string()),
        );
        data.set("shasum", Value::from(self.shasum.as_str()));
        data.set("integrity", Value::from(self.integrity.as_str()));
        data.set(
            "files",
            Value::Array(
                self.files
                    .iter()
                    .map(|f| {
                        let mut file = Value::object();
                        file.set("path", Value::from(f.path.as_str()));
                        file.set("size", Value::Number(f.size.to_string()));
                        file.set("mode", Value::Number(f.mode.to_string()));
                        file
                    })
                    .collect(),
            ),
        );
        data
    }
}

pub(super) fn build(root: &Path, data: &Value, config: &Config) -> Result<Packed> {
    let mut allow = Vec::new();
    let has_files = data.get("files").is_some();
    if let Some(files) = data.get("files") {
        let Value::Array(files) = files else {
            anyhow::bail!("package.json files must be an array");
        };
        for file in files {
            let raw = file.string().context("files entries must be strings")?;
            let (include, raw) = raw.strip_prefix('!').map_or((true, raw), |p| (false, p));
            let pattern = raw.trim_start_matches("./").trim_start_matches('/');
            ensure!(
                !pattern.contains(['\\', ':', '\0'])
                    && !pattern.split('/').any(|p| p == ".."),
                "unsafe files pattern"
            );
            if !pattern.is_empty() {
                allow.push(AllowPattern {
                    include,
                    matcher: Pattern::new(pattern)
                        .with_context(|| format!("invalid files glob: {pattern}"))?,
                });
            }
        }
    }
    let mut forced = vec!["package.json".to_owned()];
    if let Some(main) = data.str("main") {
        forced.push(relative(main)?);
    }
    for (_, bin) in scripts::bins(data) {
        forced.push(relative(&bin)?);
    }
    let mut files = Vec::new();
    walk(
        root,
        root,
        &mut Vec::new(),
        has_files,
        &allow,
        &forced,
        &mut files,
    )?;
    // A custom --userconfig/keyfile may have an arbitrary name inside the source.
    let credentials: std::collections::BTreeSet<_> = config
        .credential_files()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect();
    files.retain(|file| !credentials.contains(&root.join(&file.path)));
    ensure!(
        files.iter().any(|f| f.path == "package.json"),
        "package.json must be a regular file, not a symlink"
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for (_, bin) in scripts::bins(data) {
        let bin = relative(&bin)?;
        if let Some(file) = files.iter_mut().find(|f| f.path == bin) {
            file.mode = 0o755;
        }
    }
    let mut archive = tar::Builder::new(
        flate2::GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), flate2::Compression::default()),
    );
    for file in &files {
        // Recheck after enumeration, before opening; never intentionally follow a link.
        let path = root.join(&file.path);
        ensure!(
            fs::symlink_metadata(&path)?.is_file(),
            "package file changed during packing"
        );
        let bytes = fs::read(&path).with_context(|| format!("could not read {}", file.path))?;
        ensure!(
            bytes.len() as u64 == file.size,
            "package file changed during packing"
        );
        let mut header = tar::Header::new_gnu();
        header.set_size(file.size);
        header.set_mode(file.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        header.set_cksum();
        archive.append_data(
            &mut header,
            format!("package/{}", file.path),
            bytes.as_slice(),
        )?;
    }
    let bytes = archive.into_inner()?.finish()?;
    let name = data.str("name").unwrap();
    let version = data.str("version").unwrap();
    Ok(Packed {
        id: format!("{name}@{version}"),
        filename: format!(
            "{}-{version}.tgz",
            name.trim_start_matches('@').replace('/', "-")
        ),
        integrity: format!("sha512-{}", STANDARD.encode(Sha512::digest(&bytes))),
        shasum: format!("{:x}", sha1::Sha1::digest(&bytes)),
        bytes,
        files,
    })
}

fn relative(path: &str) -> Result<String> {
    let path = Path::new(path);
    ensure!(
        !path.is_absolute()
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "main/bin must be safe relative package paths"
    );
    let text = path
        .to_str()
        .context("non-UTF-8 package path")?
        .trim_start_matches("./");
    ensure!(
        !text.contains(['\\', ':']) && !text.chars().any(char::is_control),
        "unsafe main/bin path"
    );
    Ok(text.into())
}

fn excluded(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        ".git"
            | ".hg"
            | ".svn"
            | ".tnpm"
            | "node_modules"
            | ".npmrc"
            | ".npmignore"
            | ".gitignore"
            | ".ds_store"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "tnpm-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "npm-debug.log"
            | "id_rsa"
            | "id_ed25519"
    ) || lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".tgz")
}

fn walk(
    root: &Path,
    dir: &Path,
    ignores: &mut Vec<Gitignore>,
    has_files: bool,
    allow: &[AllowPattern],
    forced: &[String],
    files: &mut Vec<File>,
) -> Result<()> {
    let before = ignores.len();
    if !has_files || dir != root {
        let path = if dir.join(".npmignore").exists() {
            dir.join(".npmignore")
        } else {
            dir.join(".gitignore")
        };
        if let Ok(meta) = fs::symlink_metadata(&path)
            && meta.is_file()
        {
            let mut builder = GitignoreBuilder::new(dir);
            if let Some(error) = builder.add(&path) {
                return Err(error).context("invalid package ignore file");
            }
            ignores.push(builder.build()?);
        }
    }
    let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().context("package filenames must be UTF-8")?;
        ensure!(
            !name.contains(['\\', ':']) && !name.chars().any(char::is_control),
            "unsafe package filename"
        );
        if excluded(name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("non-UTF-8 package path")?
            .replace('\\', "/");
        let lower = name.to_ascii_lowercase();
        let always = forced.contains(&relative)
            || (dir == root
                && ["readme", "license", "licence", "copying"]
                    .iter()
                    .any(|p| lower == *p || lower.starts_with(&format!("{p}."))));
        let ignored = ignores
            .iter()
            .rev()
            .find_map(|matcher| {
                let result = matcher.matched_path_or_any_parents(&path, metadata.is_dir());
                if result.is_none() {
                    None
                } else {
                    Some(result.is_ignore())
                }
            })
            .unwrap_or(false);
        if metadata.is_dir() {
            let required = forced
                .iter()
                .any(|p| p.starts_with(&format!("{relative}/")));
            if !ignored || required {
                walk(root, &path, ignores, has_files, allow, forced, files)?;
            }
        } else if metadata.is_file()
            && (always
                || (!ignored
                    && (!has_files || allowlisted(&relative, allow))))
        {
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                metadata.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            files.push(File {
                path: relative,
                size: metadata.len(),
                mode: if executable { 0o755 } else { 0o644 },
            });
        }
    }
    ignores.truncate(before);
    Ok(())
}
