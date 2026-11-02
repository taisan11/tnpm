//! Human-readable `view` output. JSON mode bypasses this formatter entirely.
use crate::json::Value;
use std::{
    fmt::Write,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn summary(package: &Value, now: SystemTime) -> String {
    let mut output = String::new();
    let dependencies = package
        .get("dependencies")
        .map(Value::members)
        .unwrap_or_default();
    let versions = match package.get("versions") {
        Some(Value::Array(versions)) => versions.len(),
        Some(versions) => versions.members().len(),
        None => 0,
    };
    let license = package
        .get("license")
        .and_then(|v| v.string().or_else(|| v.str("type")))
        .unwrap_or("Proprietary");
    let dep_count = if dependencies.is_empty() {
        "none".into()
    } else {
        dependencies.len().to_string()
    };
    writeln!(
        output,
        "{}@{} | {license} | deps: {dep_count} | versions: {versions}",
        package.str("name").unwrap_or("(unknown)"),
        package.str("version").unwrap_or("(unknown)")
    )
    .unwrap();
    for key in ["description", "homepage"] {
        if let Some(text) = package.str(key).filter(|s| !s.is_empty()) {
            writeln!(output, "{text}").unwrap();
        }
    }
    if let Some(reason) = package.str("deprecated").filter(|s| !s.is_empty()) {
        writeln!(output, "\nDEPRECATED: {reason}").unwrap();
    }
    let keywords = match package.get("keywords") {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::string)
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::String(value)) => value.clone(),
        _ => String::new(),
    };
    if !keywords.is_empty() {
        writeln!(output, "\nkeywords: {keywords}").unwrap();
    }

    if let Some(dist) = package.get("dist") {
        let mut lines = Vec::new();
        for key in ["tarball", "shasum", "integrity"] {
            if let Some(value) = dist.str(key) {
                lines.push(format!(".{key}: {value}"));
            }
        }
        if let Some(Value::Number(size)) = dist.get("unpackedSize")
            && let Ok(size) = size.parse::<u64>()
        {
            lines.push(format!(".unpackedSize: {}", byte_size(size)));
        }
        if !lines.is_empty() {
            writeln!(output, "\ndist\n{}", lines.join("\n")).unwrap();
        }
    }
    if !dependencies.is_empty() {
        let items = dependencies
            .iter()
            .take(24)
            .map(|(name, version)| format!("{name}: {}", version.display()))
            .collect::<Vec<_>>();
        writeln!(output, "\ndependencies:\n{}", items.join(", ")).unwrap();
        more(&mut output, dependencies.len(), 24);
    }
    if let Some(Value::Array(maintainers)) = package.get("maintainers") {
        let maintainers = maintainers
            .iter()
            .filter_map(person)
            .map(|p| format!("- {p}"))
            .collect::<Vec<_>>();
        if !maintainers.is_empty() {
            writeln!(output, "\nmaintainers:\n{}", maintainers.join("\n")).unwrap();
        }
    }
    let tags = package
        .get("dist-tags")
        .map(Value::members)
        .unwrap_or_default();
    if !tags.is_empty() {
        writeln!(output, "\ndist-tags:").unwrap();
        for (tag, version) in tags.iter().take(5) {
            writeln!(output, "{tag}: {}", version.display()).unwrap();
        }
        more(&mut output, tags.len(), 5);
    }
    let published = package
        .str("version")
        .and_then(|v| package.get("time").and_then(|t| t.str(v)));
    let publisher = package.get("_npmUser").and_then(person);
    if published.is_some() || publisher.is_some() {
        write!(output, "\npublished").unwrap();
        if let Some(date) = published {
            let when = timestamp(date)
                .and_then(|date| relative_time(date, now))
                .unwrap_or_else(|| format!("on {date}"));
            write!(output, " {when}").unwrap();
        }
        if let Some(publisher) = publisher {
            write!(output, " by {publisher}").unwrap();
        }
        output.push('\n');
    }
    output
}

fn more(output: &mut String, count: usize, limit: usize) {
    if count > limit {
        writeln!(output, "(...and {} more.)", count - limit).unwrap();
    }
}

fn person(value: &Value) -> Option<String> {
    if let Some(value) = value.string() {
        return (!value.is_empty()).then(|| value.into());
    }
    match (value.str("name"), value.str("email")) {
        (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
        (Some(name), None) => Some(name.into()),
        (None, Some(email)) => Some(format!("<{email}>")),
        _ => None,
    }
}

fn byte_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = "B";
    for next in ["kB", "MB", "GB", "TB", "PB", "EB"] {
        size /= 1000.0;
        unit = next;
        if size < 1000.0 {
            break;
        }
    }
    format!("{size:.1} {unit}")
}

fn relative_time(published: i64, now: SystemTime) -> Option<String> {
    let now = i64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let seconds = now.abs_diff(published);
    let duration = match seconds {
        0..60 => "a few seconds".into(),
        60..120 => "a minute".into(),
        120..3600 => format!("{} minutes", seconds / 60),
        3600..7200 => "an hour".into(),
        7200..86400 => format!("{} hours", seconds / 3600),
        86400..172800 => "a day".into(),
        172800..2592000 => format!("{} days", seconds / 86400),
        2592000..5184000 => "a month".into(),
        5184000..31536000 => format!("{} months", seconds / 2592000),
        31536000..63072000 => "a year".into(),
        _ => format!("{} years", seconds / 31536000),
    };
    Some(if published > now {
        format!("in {duration}")
    } else {
        format!("{duration} ago")
    })
}

// Registry timestamps use RFC 3339. Parse them without adding a date/time dependency.
fn timestamp(text: &str) -> Option<i64> {
    let number = |start, end| -> Option<i64> {
        let part = text.get(start..end)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    for (index, expected) in [(4, "-"), (7, "-"), (10, "T"), (13, ":"), (16, ":")] {
        if text.get(index..index + 1)? != expected {
            return None;
        }
    }
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let hour = number(11, 13)?;
    let minute = number(14, 16)?;
    let second = number(17, 19)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return None,
    };
    if day < 1 || day > days || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let mut zone = text.get(19..)?;
    if let Some(fraction) = zone.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        zone = fraction.get(digits..)?;
    }
    let offset = if zone == "Z" {
        0
    } else {
        if zone.len() != 6 || !zone.starts_with(['+', '-']) || zone.get(3..4)? != ":" {
            return None;
        }
        let hours: i64 = zone.get(1..3)?.parse().ok()?;
        let minutes: i64 = zone.get(4..6)?.parse().ok()?;
        if !(0..=23).contains(&hours) || !(0..=59).contains(&minutes) {
            return None;
        }
        (hours * 3600 + minutes * 60) * if zone.starts_with('-') { -1 } else { 1 }
    };
    // Gregorian civil date -> number of days since 1970-01-01.
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some((era * 146097 + day_of_era - 719468) * 86400 + hour * 3600 + minute * 60 + second - offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn formats_npm_summary() {
        let package = Value::parse(r#"{
            "name":"@astrojs/cloudflare","version":"14.4.2","license":"MIT",
            "description":"Deploy your site to Cloudflare Workers",
            "homepage":"https://docs.astro.build/en/guides/integrations-guide/cloudflare/",
            "keywords":["withastro","astro-adapter"],"versions":["14.4.1","14.4.2"],
            "dependencies":{"vite":"^8.3.4","prismjs":"^1.30.0"},
            "dist":{"tarball":"https://example.test/cloudflare.tgz","shasum":"abc","integrity":"sha512-xyz","unpackedSize":132300},
            "maintainers":[{"name":"fredkschott","email":"fkschott@gmail.com"}],
            "dist-tags":{"latest":"14.4.2","beta":"15.0.0-beta.3","alpha":"14.0.0-alpha.1","one":"1","two":"2","three":"3","four":"4"},
            "time":{"14.4.2":"2026-10-21T00:00:00.000Z"},
            "_npmUser":{"name":"GitHub Actions","email":"npm-oidc-no-reply@github.com"},"_internal":"hidden"
        }"#).unwrap();
        let now =
            UNIX_EPOCH + Duration::from_secs(timestamp("2026-10-23T00:00:00Z").unwrap() as u64);
        assert_eq!(
            summary(&package, now),
            "@astrojs/cloudflare@14.4.2 | MIT | deps: 2 | versions: 2\nDeploy your site to Cloudflare Workers\nhttps://docs.astro.build/en/guides/integrations-guide/cloudflare/\n\nkeywords: withastro, astro-adapter\n\ndist\n.tarball: https://example.test/cloudflare.tgz\n.shasum: abc\n.integrity: sha512-xyz\n.unpackedSize: 132.3 kB\n\ndependencies:\nvite: ^8.3.4, prismjs: ^1.30.0\n\nmaintainers:\n- fredkschott <fkschott@gmail.com>\n\ndist-tags:\nlatest: 14.4.2\nbeta: 15.0.0-beta.3\nalpha: 14.0.0-alpha.1\none: 1\ntwo: 2\n(...and 2 more.)\n\npublished 2 days ago by GitHub Actions <npm-oidc-no-reply@github.com>\n"
        );
    }

    #[test]
    fn handles_sparse_metadata_and_legacy_license() {
        let package = Value::parse(r#"{"name":"minimal","version":"1.0.0","license":{"type":"BSD-2-Clause"},"versions":["1.0.0"]}"#).unwrap();
        assert_eq!(
            summary(&package, UNIX_EPOCH),
            "minimal@1.0.0 | BSD-2-Clause | deps: none | versions: 1\n"
        );
    }

    #[test]
    fn parses_registry_timestamps_and_timezones() {
        assert_eq!(timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(timestamp("1970-01-01T01:30:00.123+01:30"), Some(0));
        assert_eq!(timestamp("1969-12-31T23:00:00-01:00"), Some(0));
        assert_eq!(timestamp("2000-02-29T00:00:00Z"), Some(951782400));
        for bad in [
            "not-a-date",
            "2026-02-29T00:00:00Z",
            "2026-10-23T25:00:00Z",
            "2026-10-23T00:00:00.Z",
            "2026-10-23T00:00:00+99:00",
        ] {
            assert_eq!(timestamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_sizes_and_future_publications() {
        assert_eq!(byte_size(0), "0 B");
        assert_eq!(byte_size(999), "999 B");
        assert_eq!(byte_size(1234567), "1.2 MB");
        assert_eq!(relative_time(172800, UNIX_EPOCH), Some("in 2 days".into()));
    }
}
