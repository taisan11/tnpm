//! Local packing, publication, and distribution-tag management.
use super::registry;
use crate::{cli::Options, config::Config, json::Value, manifest, scripts, semver, spec};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Method;
use std::{fs, io::Write, path::Path};

mod pack;

pub(crate) async fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    if options.command == "dist-tag" {
        return dist_tag(root, options, config).await;
    }
    let source = root.join(options.arguments.first().map(String::as_str).unwrap_or("."));
    ensure!(
        source.is_dir(),
        "pack/publish currently accept local directories only"
    );
    let source = source.canonicalize()?;
    ensure!(
        fs::symlink_metadata(source.join("package.json"))?.is_file(),
        "package.json must be a regular file, not a symlink"
    );
    let mut data = manifest::read(&source)?.context("pack/publish require package.json")?;
    validate(&data, options.command == "publish")?;
    let scripts_enabled = !options.ignore_scripts && !config.enabled("ignore-scripts");
    if scripts_enabled {
        if options.command == "publish" {
            scripts::lifecycle_output(&source, &data, "prepublishOnly", options.json)?;
        }
        scripts::lifecycle_output(&source, &data, "prepack", options.json)?;
        scripts::lifecycle_output(&source, &data, "prepare", options.json)?;
        // Build hooks may update both the manifest and the file list.
        data =
            manifest::read(&source)?.context("package.json was removed by a lifecycle script")?;
        validate(&data, options.command == "publish")?;
    }
    let packed = pack::build(&source, &data, config)?;
    if scripts_enabled {
        scripts::lifecycle_output(&source, &data, "postpack", options.json)?;
    }
    let dry_run = options.dry_run || config.enabled("dry-run");
    if options.command == "pack" {
        if !dry_run {
            let destination = options
                .pack_destination
                .as_deref()
                .unwrap_or(Path::new("."));
            fs::create_dir_all(destination)?;
            let path = destination.join(&packed.filename);
            let mut temp = tempfile::NamedTempFile::new_in(destination)?;
            temp.write_all(&packed.bytes)?;
            temp.as_file().sync_all()?;
            temp.persist(&path)
                .map_err(|e| e.error)
                .context("could not save package tarball")?;
        }
        report(&packed, options, dry_run, true);
        return Ok(());
    }
    let publish_config = data.get("publishConfig");
    let setting = |key| publish_config.and_then(|v| v.str(key));
    ensure!(
        !config.enabled("provenance") && config.get("provenance-file").is_none_or(str::is_empty),
        "publish provenance is not supported"
    );
    let name = data.str("name").unwrap();
    let version = data.str("version").unwrap();
    let tag = options
        .tag
        .as_deref()
        .or_else(|| setting("tag"))
        .or_else(|| config.get("tag"))
        .unwrap_or("latest");
    valid_tag(tag)?;
    let access = options
        .access
        .as_deref()
        .or_else(|| setting("access"))
        .or_else(|| config.get("access"));
    ensure!(
        access.is_none_or(|a| matches!(a, "public" | "restricted")),
        "access must be public or restricted"
    );
    ensure!(
        name.starts_with('@') || access != Some("restricted"),
        "restricted access requires a scoped package"
    );
    let url = config.registry_url(
        name,
        options.registry.as_deref().or_else(|| setting("registry")),
    )?;
    if !dry_run {
        registry::require_auth(config, &url)?;
        let body = metadata(&data, &packed, &url, tag, access);
        let response =
            registry::request(config, options, Method::PUT, &url, Some(&body), true).await?;
        registry::check(&response)?;
        if scripts_enabled {
            scripts::lifecycle_output(&source, &data, "publish", options.json)?;
            scripts::lifecycle_output(&source, &data, "postpublish", options.json)?;
        }
    }
    eprintln!(
        "{} {name}@{version} with tag {tag}",
        if dry_run {
            "Would publish"
        } else {
            "Published"
        }
    );
    report(&packed, options, dry_run, false);
    Ok(())
}

fn validate(data: &Value, publishing: bool) -> Result<()> {
    let name = data.str("name").context("package.json requires a name")?;
    valid_name(name)?;
    let version = data
        .str("version")
        .context("package.json requires a version")?;
    ensure!(
        semver::Version::parse(version).is_some_and(|v| v.to_string() == version),
        "package version must be a canonical SemVer"
    );
    if publishing {
        ensure!(
            data.get("private").is_none_or(|v| v == &Value::Bool(false)),
            "private packages cannot be published (including --dry-run)"
        );
    }
    for key in ["bundleDependencies", "bundledDependencies"] {
        ensure!(
            match data.get(key) {
                None | Some(Value::Bool(false)) => true,
                Some(Value::Array(a)) => a.is_empty(),
                _ => false,
            },
            "packing bundled dependencies is not supported"
        );
    }
    if publishing {
        for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
            for (_, value) in data.get(field).map(Value::members).unwrap_or_default() {
                ensure!(
                    !value.string().is_some_and(|s| s.starts_with("workspace:")),
                    "replace workspace: dependency ranges before publishing"
                );
            }
        }
    }
    if let Some(publish_config) = data.get("publishConfig") {
        ensure!(
            matches!(publish_config, Value::Object(_)),
            "publishConfig must be an object"
        );
        for (key, value) in publish_config.members() {
            ensure!(
                matches!(key.as_str(), "registry" | "tag" | "access") && value.string().is_some(),
                "unsupported publishConfig field; only registry, tag and access strings are supported"
            );
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> Result<()> {
    ensure!(
        crate::lockfile::valid_name(name)
            && name.len() <= 214
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"@/-._~".contains(&b))
            && name
                .trim_start_matches('@')
                .split('/')
                .all(|p| !p.starts_with(['.', '_']))
            && !matches!(name, "node_modules" | "favicon.ico"),
        "invalid publish package name"
    );
    Ok(())
}

fn valid_tag(tag: &str) -> Result<()> {
    ensure!(
        !tag.is_empty()
            && !matches!(tag, "." | "..")
            && tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && semver::satisfies("0.0.0", tag).is_none(),
        "invalid dist-tag; tags must not be SemVer versions or ranges"
    );
    Ok(())
}

fn metadata(
    data: &Value,
    packed: &pack::Packed,
    url: &reqwest::Url,
    tag: &str,
    access: Option<&str>,
) -> Value {
    let name = data.str("name").unwrap();
    let version = data.str("version").unwrap();
    let mut version_data = data.clone();
    version_data.set("_id", Value::from(format!("{name}@{version}").as_str()));
    let mut dist = Value::object();
    dist.set("integrity", Value::from(packed.integrity.as_str()));
    dist.set("shasum", Value::from(packed.shasum.as_str()));
    // The path uses the unescaped scope separator, unlike the document endpoint.
    let mut base = url.clone();
    let attachment_name = format!("{name}-{version}.tgz");
    base.path_segments_mut()
        .unwrap()
        .pop()
        .extend(name.split('/'))
        .push("-")
        .extend(attachment_name.split('/'));
    dist.set("tarball", Value::from(base.as_str()));
    version_data.set("dist", dist);
    let mut versions = Value::object();
    versions.set(version, version_data);
    let mut tags = Value::object();
    tags.set(tag, Value::from(version));
    let mut attachment = Value::object();
    attachment.set("content_type", Value::from("application/octet-stream"));
    attachment.set("data", Value::from(STANDARD.encode(&packed.bytes).as_str()));
    attachment.set("length", Value::Number(packed.bytes.len().to_string()));
    let mut attachments = Value::object();
    // npm uses the scoped name (including '/') as the attachment key.
    attachments.set(&attachment_name, attachment);
    let mut body = Value::object();
    body.set("_id", Value::from(name));
    body.set("name", Value::from(name));
    if let Some(description) = data.get("description") {
        body.set("description", description.clone());
    }
    body.set("dist-tags", tags);
    body.set("versions", versions);
    body.set("access", access.map(Value::from).unwrap_or(Value::Null));
    body.set("_attachments", attachments);
    body
}

fn report(packed: &pack::Packed, options: &Options, dry_run: bool, packing: bool) {
    if options.json {
        let mut summary = packed.summary();
        summary.set("dryRun", Value::Bool(dry_run));
        println!(
            "{}",
            if packing {
                Value::Array(vec![summary])
            } else {
                summary
            }
            .pretty()
        );
    } else {
        for file in &packed.files {
            eprintln!("  {} ({} bytes)", file.path, file.size);
        }
        if packing {
            println!("{}", packed.filename);
        } else {
            println!("+ {}{}", packed.id, if dry_run { " (dry-run)" } else { "" });
        }
    }
}

async fn dist_tag(root: &Path, options: &Options, config: &Config) -> Result<()> {
    ensure!(
        !options.dry_run && !config.enabled("dry-run"),
        "dist-tag does not support --dry-run"
    );
    ensure!(
        options.passthrough.is_empty(),
        "unexpected dist-tag arguments"
    );
    let args = &options.arguments;
    let action = args.first().map(String::as_str).unwrap_or("ls");
    let (name, version, tag, method) = match action {
        "ls" | "list" if args.len() <= 2 => {
            let name = args
                .get(1)
                .cloned()
                .or_else(|| {
                    manifest::read(root)
                        .ok()
                        .flatten()
                        .and_then(|m| m.str("name").map(str::to_owned))
                })
                .context("dist-tag ls requires a package name or package.json")?;
            (name, None, None, Method::GET)
        }
        "add" if (2..=3).contains(&args.len()) => {
            let (name, version) = spec::package_spec(&args[1])?;
            let version = version.context("dist-tag add requires package@version")?;
            ensure!(
                semver::Version::parse(version).is_some_and(|v| v.to_string() == version),
                "dist-tag add requires an exact canonical version"
            );
            let tag = args
                .get(2)
                .map(String::as_str)
                .or(options.tag.as_deref())
                .or_else(|| config.get("tag"))
                .unwrap_or("latest");
            (
                name.to_owned(),
                Some(version.to_owned()),
                Some(tag.to_owned()),
                Method::PUT,
            )
        }
        "rm" | "remove" if args.len() == 3 => {
            (args[1].clone(), None, Some(args[2].clone()), Method::DELETE)
        }
        _ => anyhow::bail!(
            "usage: tnpm dist-tag <ls [PACKAGE]|add PACKAGE@VERSION [TAG]|rm PACKAGE TAG>"
        ),
    };
    valid_name(&name)?;
    if let Some(tag) = &tag {
        valid_tag(tag)?;
    }
    let package_url = config.registry_url(&name, options.registry.as_deref())?;
    let mut base = package_url.clone();
    base.path_segments_mut().unwrap().pop().push("");
    let mut segments = vec!["-", "package", name.as_str(), "dist-tags"];
    if let Some(tag) = &tag {
        segments.push(tag);
    }
    let url = registry::endpoint(&base, &segments);
    if method != Method::GET {
        registry::require_auth(config, &url)?;
    }
    let body = version.as_deref().map(Value::from);
    let response =
        registry::request(config, options, method.clone(), &url, body.as_ref(), true).await?;
    if method == Method::GET {
        let data = registry::json(response).await?;
        ensure!(
            matches!(data, Value::Object(_)),
            "invalid dist-tag response"
        );
        if options.json {
            println!("{}", data.pretty());
        } else {
            let mut tags: Vec<_> = data.members().iter().collect();
            tags.sort_by_key(|(key, _)| key);
            for (tag, version) in tags {
                println!("{tag}: {}", version.display());
            }
        }
    } else {
        registry::check(&response)?;
        if options.json {
            println!("{{\"ok\":true}}");
        } else {
            println!(
                "{} {}:{}",
                if method == Method::PUT { "+" } else { "-" },
                name,
                tag.unwrap()
            );
        }
    }
    Ok(())
}
