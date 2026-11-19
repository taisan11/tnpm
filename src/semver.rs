//! SemVer parsing, ordering, release increments, and npm-compatible ranges.
//!
//! Version parsing and ordering are delegated to the [`semver`] crate. npm's
//! range grammar differs from Cargo's (`1.2` means `1.2.x`, spaces combine
//! comparators, `||` unions ranges, and hyphens are inclusive), so ranges are
//! translated to `semver::VersionReq` comparator groups before evaluation.
use anyhow::{Context, Result, bail};
use semver::{BuildMetadata, Prerelease, Version as SemverVersion, VersionReq};
use std::cmp::Ordering;
use std::fmt;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(SemverVersion);

impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('v').unwrap_or(value);
        SemverVersion::parse(value).ok().map(Self)
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }

    pub fn bump(&self, release: &str, preid: &str) -> Result<Self> {
        if let Some(version) = Self::parse(release) {
            return Ok(version);
        }
        if !preid.is_empty() && Prerelease::new(preid).is_err() {
            bail!("invalid --preid");
        }
        let mut version = self.0.clone();
        version.build = BuildMetadata::EMPTY;
        let inc = |n: u64| n.checked_add(1).context("version number overflow");
        match release {
            "major" => {
                if version.minor != 0 || version.patch != 0 || version.pre.is_empty() {
                    version.major = inc(version.major)?;
                }
                version.minor = 0;
                version.patch = 0;
                version.pre = Prerelease::EMPTY;
            }
            "minor" => {
                if version.patch != 0 || version.pre.is_empty() {
                    version.minor = inc(version.minor)?;
                }
                version.patch = 0;
                version.pre = Prerelease::EMPTY;
            }
            "patch" => {
                if version.pre.is_empty() {
                    version.patch = inc(version.patch)?;
                }
                version.pre = Prerelease::EMPTY;
            }
            "premajor" => {
                version.major = inc(version.major)?;
                version.minor = 0;
                version.patch = 0;
                version.pre = Prerelease::EMPTY;
            }
            "preminor" => {
                version.minor = inc(version.minor)?;
                version.patch = 0;
                version.pre = Prerelease::EMPTY;
            }
            "prepatch" => {
                version.patch = inc(version.patch)?;
                version.pre = Prerelease::EMPTY;
            }
            "prerelease" => {
                if version.pre.is_empty() {
                    version.patch = inc(version.patch)?;
                }
            }
            _ => bail!("invalid version or release type: {release}"),
        }
        if release.starts_with("pre") {
            let mut identifiers: Vec<String> = if version.pre.is_empty() {
                Vec::new()
            } else {
                version.pre.as_str().split('.').map(str::to_owned).collect()
            };
            if !preid.is_empty() {
                let prefix: Vec<&str> = preid.split('.').collect();
                let matches_prefix = identifiers.len() > prefix.len()
                    && identifiers
                        .iter()
                        .map(String::as_str)
                        .zip(&prefix)
                        .all(|(a, b)| a == *b);
                if !matches_prefix {
                    identifiers = prefix.into_iter().map(str::to_owned).collect();
                    identifiers.push("0".into());
                    version.pre =
                        Prerelease::new(&identifiers.join(".")).context("invalid --preid")?;
                    return Ok(Self(version));
                }
            }
            if let Some(index) = identifiers
                .iter()
                .rposition(|part| part.bytes().all(|b| b.is_ascii_digit()))
            {
                let old: u64 = identifiers[index]
                    .parse()
                    .context("prerelease number overflow")?;
                identifiers[index] = inc(old)?.to_string();
            } else {
                identifiers.push("0".into());
            }
            version.pre = Prerelease::new(&identifiers.join(".")).context("invalid --preid")?;
        }
        Ok(Self(version))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Evaluate a registry version against npm's range syntax.
///
/// Non-registry specs (file:, git URLs, aliases, tags) return `None`.
pub fn satisfies(version: &str, range: &str) -> Option<bool> {
    let version = parse_version(version)?;
    let requests = parse_range(range)?;
    Some(requests.iter().any(|request| request.matches(&version)))
}

/// Ranges are unions of comparator intervals. An inhabited intersection has a
/// witness at a lower endpoint or its immediate stable/prerelease successor.
pub fn intersects(left: &str, right: &str) -> Option<bool> {
    satisfies("0.0.0", left)?;
    satisfies("0.0.0", right)?;
    let mut candidates = vec![SemverVersion::new(0, 0, 0)];
    for range in [left, right] {
        for text in range.split_whitespace().filter(|s| *s != "||" && *s != "-") {
            let text = text.trim_start_matches(['>', '<', '=', '^', '~']);
            if text.is_empty() {
                continue;
            }
            let Some((base, count)) = partial_base(text) else {
                continue;
            };
            candidates.push(base.clone());
            if !base.pre.is_empty() {
                let mut next = base.clone();
                next.pre = Prerelease::new(&format!("{}.0", base.pre))
                    .context("prerelease is invalid")
                    .ok()?;
                candidates.push(next);
                let mut stable = base.clone();
                stable.pre = Prerelease::EMPTY;
                candidates.push(stable);
            }
            if count < 3 {
                if let Some(next) = upper(base.clone(), count) {
                    candidates.push(next);
                }
            } else if let Some(patch) = base.patch.checked_add(1) {
                let mut next = base;
                next.patch = patch;
                next.pre = Prerelease::EMPTY;
                candidates.push(next);
            }
        }
    }
    Some(candidates.iter().any(|candidate| {
        let value = candidate.to_string();
        satisfies(&value, left) == Some(true) && satisfies(&value, right) == Some(true)
    }))
}

fn parse_version(value: &str) -> Option<SemverVersion> {
    let value = value.strip_prefix('v').unwrap_or(value);
    SemverVersion::parse(value).ok()
}

fn parse_range(range: &str) -> Option<Vec<VersionReq>> {
    let range = range.trim();
    if range.is_empty() {
        return Some(vec![VersionReq::STAR]);
    }
    range
        .split("||")
        .map(|group| parse_group(group.trim()))
        .collect()
}

fn parse_group(group: &str) -> Option<VersionReq> {
    if group.is_empty() {
        return Some(VersionReq::STAR);
    }
    let tokens: Vec<&str> = group.split_whitespace().collect();
    let mut comparators = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if index + 2 < tokens.len() && tokens[index + 1] == "-" {
            let (low, low_count) = partial(tokens[index])?;
            let (high, high_count) = partial(tokens[index + 2])?;
            if low_count > 0 {
                comparators.push(format!(">={}", partial_version(&low, low_count)?));
            }
            if high_count > 0 {
                if high_count == 3 {
                    comparators.push(format!("<={}", partial_version(&high, high_count)?));
                } else {
                    comparators.push(format!("<{}", upper(high, high_count)?));
                }
            }
            index += 3;
            continue;
        }
        let (operator, text, used_next) = split_operator(tokens[index], tokens.get(index + 1))?;
        let (version, count) = partial(text)?;
        comparators.extend(expand(operator, &version, count)?);
        index += if used_next { 2 } else { 1 };
    }
    let text = comparators.join(", ");
    if text.is_empty() {
        return Some(VersionReq::STAR);
    }
    VersionReq::parse(&text).ok()
}

fn split_operator<'a>(token: &'a str, next: Option<&&'a str>) -> Option<(&'a str, &'a str, bool)> {
    for operator in [">=", "<=", ">", "<", "^", "~>", "~", "="] {
        if let Some(rest) = token.strip_prefix(operator) {
            let operator = if operator == "~>" { "~" } else { operator };
            if rest.is_empty() {
                return Some((operator, next.copied()?, true));
            }
            return Some((operator, rest, false));
        }
    }
    Some(("", token, false))
}

fn expand(operator: &str, version: &SemverVersion, count: usize) -> Option<Vec<String>> {
    if count == 0 {
        return Some(match operator {
            ">" | "<" => vec!["<0.0.0-0".into()],
            _ => Vec::new(),
        });
    }
    let lower = partial_version(version, count)?;
    let upper = upper(version.clone(), count);
    Some(match operator {
        "^" | "~" => vec![format!("{operator}{}", partial_shorthand(version, count)?)],
        ">" => {
            if count == 3 {
                vec![format!(">{lower}")]
            } else {
                vec![format!(">={}", upper?)]
            }
        }
        ">=" => vec![format!(">={lower}")],
        "<" => {
            if count == 3 {
                vec![format!("<{lower}")]
            } else {
                vec![format!("<{lower}-0")]
            }
        }
        "<=" => {
            if count == 3 {
                vec![format!("<={lower}")]
            } else {
                vec![format!("<{}", upper_marker(version.clone(), count)?)]
            }
        }
        "=" | "" => {
            if count == 3 {
                vec![format!("={lower}")]
            } else {
                vec![format!(">={lower}"), format!("<{}", upper?)]
            }
        }
        _ => return None,
    })
}

fn partial(value: &str) -> Option<(SemverVersion, usize)> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let value = value.split('+').next()?;
    if let Ok(version) = SemverVersion::parse(value) {
        return Some((version, 3));
    }
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() > 3 {
        return None;
    }
    let mut numbers = [0; 3];
    let mut count = 0;
    let mut wildcard = false;
    for (index, part) in parts.iter().enumerate() {
        if matches!(*part, "*" | "x" | "X" | "") {
            wildcard = true;
            continue;
        }
        if wildcard
            || !part.bytes().all(|c| c.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return None;
        }
        numbers[index] = part.parse().ok()?;
        count += 1;
    }
    Some((
        SemverVersion::new(numbers[0], numbers[1], numbers[2]),
        count,
    ))
}

fn partial_base(value: &str) -> Option<(SemverVersion, usize)> {
    partial(value)
}

/// Render only the components present in a partial range (`~1.2`, not `~1.2.0`).
fn partial_shorthand(version: &SemverVersion, count: usize) -> Option<String> {
    if count == 0 {
        return None;
    }
    let mut output = version.major.to_string();
    if count >= 2 {
        let _ = write!(output, ".{}", version.minor);
    }
    if count >= 3 {
        let _ = write!(output, ".{}", version.patch);
        if !version.pre.is_empty() {
            let _ = write!(output, "-{}", version.pre);
        }
    }
    Some(output)
}

fn partial_version(version: &SemverVersion, count: usize) -> Option<String> {
    let mut version = version.clone();
    version.build = BuildMetadata::EMPTY;
    if count < 3 {
        version.pre = Prerelease::EMPTY;
    }
    Some(version.to_string())
}

fn upper(mut version: SemverVersion, count: usize) -> Option<SemverVersion> {
    match count {
        0 => return None,
        1 => {
            version.major = version.major.checked_add(1)?;
            version.minor = 0;
            version.patch = 0;
        }
        _ => {
            version.minor = version.minor.checked_add(1)?;
            version.patch = 0;
        }
    }
    version.pre = Prerelease::EMPTY;
    version.build = BuildMetadata::EMPTY;
    Some(version)
}

/// npm excludes prereleases at a partial upper bound (`<2.0.0-0`).
fn upper_marker(version: SemverVersion, count: usize) -> Option<String> {
    let mut upper = upper(version, count)?;
    upper.pre = Prerelease::new("0").ok()?;
    Some(upper.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_intersections_include_partial_strict_or_and_prerelease_bounds() {
        for (a, b, expected) in [
            ("^1", "1", true),
            ("^1", "^2", false),
            (">1", ">=2 <3", true),
            (">1.0.0 <1.0.1", "*", false),
            (">1.0.0-alpha <1.0.0-beta", "1.0.0-alpha.0", true),
            ("1 || 3", "^2", false),
            ("1 || 3", ">=2", true),
            ("1.0 - 1.2", ">=1.2.9", true),
        ] {
            assert_eq!(intersects(a, b), Some(expected), "{a} / {b}");
            assert_eq!(intersects(b, a), Some(expected), "{b} / {a}");
        }
        assert_eq!(intersects("latest", "*"), None);
    }
    #[test]
    fn semver_validation_and_ordering() {
        for invalid in ["1", "01.2.3", "1.2.3-01", "1.2.3-", "1.2.3+a+b"] {
            assert!(Version::parse(invalid).is_none());
        }
        let sequence = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for pair in sequence.windows(2) {
            assert_eq!(
                Version::parse(pair[0])
                    .unwrap()
                    .compare(&Version::parse(pair[1]).unwrap()),
                Ordering::Less
            );
        }
    }

    #[test]
    fn release_bumps() {
        for (old, release, preid, expected) in [
            ("1.2.3", "patch", "", "1.2.4"),
            ("1.2.3", "minor", "", "1.3.0"),
            ("1.2.3", "major", "", "2.0.0"),
            ("2.0.0-rc.1", "major", "", "2.0.0"),
            ("1.2.3-beta.1", "patch", "", "1.2.3"),
            ("1.2.3", "preminor", "beta", "1.3.0-beta.0"),
            ("1.2.3", "prerelease", "", "1.2.4-0"),
            ("1.2.4-beta.9", "prerelease", "beta", "1.2.4-beta.10"),
            ("1.2.4-beta.9", "prerelease", "rc", "1.2.4-rc.0"),
        ] {
            assert_eq!(
                Version::parse(old)
                    .unwrap()
                    .bump(release, preid)
                    .unwrap()
                    .to_string(),
                expected
            );
        }
    }

    #[test]
    fn npm_ranges() {
        for (v, range, expected) in [
            ("1.5.0", "^1.2.3", true),
            ("2.0.0", "^1.2.3", false),
            ("0.2.9", "^0.2.3", true),
            ("0.3.0", "^0.2.3", false),
            ("0.0.4", "^0.0.3", false),
            ("1.2.9", "~1.2.3", true),
            ("1.3.0", "~1.2.3", false),
            ("1.9.9", "~1", true),
            ("2.0.0", "~1", false),
            ("1.9.9", "~1.x", true),
            ("0.9.9", "^0", true),
            ("1.0.0", "^0", false),
            ("1.2.3", "~>1.2.3", true),
            ("1.3.0", "~>1.2.3", false),
            ("2.0.0", ">=1 <3", true),
            ("1.9.0", "1.x", true),
            ("2.0.0", "1.x", false),
            ("2.5.0", "1 || 2", true),
            ("2.9.0", "1.2 - 2", true),
            ("1.2.3-beta.1", "*", false),
            ("1.2.3-beta.2", ">=1.2.3-beta.1 <2", true),
            ("1.2.3", "1.2.3", true),
            ("1.2.4", "1.2.3", false),
            ("1.2.9", "=1.2", true),
            ("1.3.0", "=1.2", false),
            ("1.9.9", "1", true),
            ("2.0.0", "1", false),
            ("2.0.0", ">1", true),
            ("1.9.9", ">1", false),
            ("0.9.9", "<1", true),
            ("1.0.0", "<1", false),
            ("1.0.0-alpha", "<1", false),
            ("1.1.9", "<1.2", true),
            ("1.2.0", "<1.2", false),
            ("1.2.0-alpha", "<1.2", false),
            ("1.2.9", "<=1.2", true),
            ("1.3.0", "<=1.2", false),
            ("1.3.0-alpha", "<=1.2", false),
            ("1.2.3", "v1.2.3", true),
            ("1.2.3", "1.2.3+build", true),
            ("1.2.4", ">=1.2.3+build", true),
            ("2.4.0", "1.2.3 - 2.3", false),
            ("2.3.9", "1.2.3 - 2.3", true),
            ("3.0.0-alpha", "1.2.3 - 2", false),
            ("1.2.0", "1.2.x", true),
            ("1.3.0-alpha", "1.2.x", false),
            ("2.0.0", "^*", true),
            ("1.0.0-0", "*", false),
            ("1.0.0", ">=*", true),
            ("1.0.0-0", ">=*", false),
            ("1.0.0", "<=*", true),
            ("1.0.0-0", "<=*", false),
            ("1.2.3", "1.2.3 - 2.3.4", true),
            ("2.3.4", "1.2.3 - 2.3.4", true),
            ("2.3.5", "1.2.3 - 2.3.4", false),
            ("1.2.3-beta.1", "^1.2.3-beta.0", true),
            ("1.2.4-beta.1", "^1.2.3-beta.0", false),
        ] {
            assert_eq!(satisfies(v, range), Some(expected), "{v} {range}");
        }
        for range in ["file:../x", "latest", "1.2.3 || latest", "1.2.3-"] {
            assert_eq!(satisfies("1.0.0", range), None, "{range}");
        }
        assert_eq!(satisfies("1.0.0", ""), Some(true));
        assert_eq!(satisfies("1.0.0", "  "), Some(true));
        assert_eq!(satisfies("1.0.0", "*"), Some(true));
        assert_eq!(satisfies("1.0.0", "x"), Some(true));
    }
}
