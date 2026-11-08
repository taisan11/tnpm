use crate::{
    cli::Options, config::Config, json::Value, manifest, profile::Profile, registry,
    spec::package_spec, util::style::view as format,
};
use anyhow::{Context, Result};
use std::{path::Path, time::SystemTime};

fn field(value: &Value, path: &str) -> Option<Value> {
    // npm fields may use dot paths, array indexes, or projection (maintainers.email).
    let normalized = path.replace('[', ".").replace(']', "");
    fn walk(value: &Value, parts: &[&str]) -> Option<Value> {
        let Some((part, rest)) = parts.split_first() else {
            return Some(value.clone());
        };
        match value {
            Value::Array(values) => {
                if let Ok(index) = part.parse::<usize>() {
                    walk(values.get(index)?, rest)
                } else {
                    let values: Vec<_> = values.iter().filter_map(|v| walk(v, parts)).collect();
                    if values.is_empty() {
                        None
                    } else {
                        Some(Value::Array(values))
                    }
                }
            }
            _ => walk(value.get(part)?, rest),
        }
    }
    walk(value, &normalized.split('.').collect::<Vec<_>>())
}

fn metadata(packument: &Value, version: &Value) -> Value {
    let mut result = packument.clone();
    for (key, value) in version.members() {
        result.set(key, value.clone());
    }
    result
}

pub async fn run(root: &Path, options: &Options, config: &Config) -> Result<()> {
    let mut profile = Profile::new("view");
    let local;
    let spec = if let Some(spec) = options.arguments.first().filter(|s| s.as_str() != ".") {
        spec.as_str()
    } else {
        local = manifest::read(root)?
            .and_then(|m| m.str("name").map(str::to_owned))
            .context("provide a package name, or run view in a project with a package name")?;
        &local
    };
    let jsr;
    let (name, selector) = if spec.starts_with("jsr:") {
        let parsed = crate::jsr::Specifier::parse(spec)?;
        anyhow::ensure!(
            parsed.subpath.is_empty(),
            "view does not accept JSR export subpaths"
        );
        jsr = parsed.npm_name();
        (jsr.as_str(), Some(parsed.range))
    } else {
        package_spec(spec)?
    };
    let url = if name.starts_with("@jsr/") {
        crate::jsr::registry_url(config, name)?
    } else {
        config.registry_url(name, options.registry.as_deref())?
    };
    let authorization = config.authorization(&url)?;
    let no_cache = options.no_cache || config.authenticated(&url);
    let fields = options.arguments.get(1..).unwrap_or_default();
    // Only these top-level fields can be observed by the later projection.
    // Normalize exactly like field(), including bracket indexes and ']'.
    let roots: Vec<_> = fields
        .iter()
        .map(|field| {
            field
                .replace('[', ".")
                .replace(']', "")
                .split('.')
                .next()
                .unwrap_or("")
                .to_owned()
        })
        .collect();
    profile.mark("prepare_ms");
    let package = registry::fetch_using(
        url.clone(),
        authorization.as_ref(),
        registry::Query::view(
            selector.unwrap_or(config.get("tag").unwrap_or("latest")),
            &roots,
        ),
        no_cache,
        || config.http_client(&url, false),
        None,
    )
    .await?;
    profile.mark("registry_ms");
    let packument = package.metadata;
    let selected = package.versions;
    if fields.is_empty() && !options.json {
        let now = SystemTime::now();
        let summaries: Vec<_> = selected
            .iter()
            .map(|version| format::summary(&metadata(&packument, version), now))
            .collect();
        print!("{}", summaries.join("\n"));
        profile.mark("render_ms");
        return Ok(());
    }
    let mut output = Vec::new();
    for selected in selected {
        let data = metadata(&packument, &selected);
        if fields.is_empty() {
            output.push(data);
        } else if fields.len() == 1 {
            if let Some(value) = field(&data, &fields[0]) {
                output.push(value);
            }
        } else {
            let mut values = Value::object();
            for name in fields {
                if let Some(value) = field(&data, name) {
                    values.set(name, value);
                }
            }
            output.push(values);
        }
    }
    if output.is_empty() {
        profile.mark("render_ms");
        return Ok(());
    }
    let result = if output.len() == 1 {
        output.remove(0)
    } else {
        Value::Array(output)
    };
    if options.json {
        println!("{}", result.pretty());
    } else {
        println!("{}", result.display());
    }
    profile.mark("render_ms");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_and_projected_fields() {
        let data = Value::parse(r#"{"maintainers":[{"email":"a@test"},{"email":"b@test"}],"dist":{"tarball":"https://example.test/a.tgz"}}"#).unwrap();
        assert_eq!(
            field(&data, "maintainers[1].email"),
            Some(Value::from("b@test"))
        );
        assert_eq!(
            field(&data, "maintainers.email"),
            Some(Value::Array(vec![
                Value::from("a@test"),
                Value::from("b@test")
            ]))
        );
        assert!(field(&data, "dist.missing").is_none());
    }
}
