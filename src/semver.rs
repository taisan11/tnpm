//! SemVer parsing, ordering, release increments, and npm-compatible ranges.
use anyhow::{Context, Result, bail};
use std::cmp::Ordering;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Vec<String>,
    build: Option<String>,
}

fn identifiers(value: &str, prerelease: bool) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && (!prerelease
                    || !part.bytes().all(|c| c.is_ascii_digit())
                    || part.len() == 1
                    || !part.starts_with('0'))
        })
}

impl Version {
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.strip_prefix('v').unwrap_or(value);
        let (value, build) = match value.split_once('+') {
            Some((value, build)) if identifiers(build, false) => (value, Some(build.into())),
            Some(_) => return None,
            None => (value, None),
        };
        let (value, pre) = match value.split_once('-') {
            Some((value, pre)) if identifiers(pre, true) => {
                (value, pre.split('.').map(str::to_owned).collect())
            }
            Some(_) => return None,
            None => (value, Vec::new()),
        };
        let numbers: Vec<_> = value.split('.').collect();
        if numbers.len() != 3
            || numbers.iter().any(|p| {
                p.is_empty()
                    || !p.bytes().all(|c| c.is_ascii_digit())
                    || (p.len() > 1 && p.starts_with('0'))
            })
        {
            return None;
        }
        Some(Self {
            major: numbers[0].parse().ok()?,
            minor: numbers[1].parse().ok()?,
            patch: numbers[2].parse().ok()?,
            pre,
            build,
        })
    }

    fn core(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        let core = self.core().cmp(&other.core());
        if core != Ordering::Equal {
            return core;
        }
        if self.pre.is_empty() != other.pre.is_empty() {
            return if self.pre.is_empty() {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }
        for (a, b) in self.pre.iter().zip(&other.pre) {
            let an = a.bytes().all(|c| c.is_ascii_digit());
            let bn = b.bytes().all(|c| c.is_ascii_digit());
            let order = match (an, bn) {
                (true, true) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => a.cmp(b),
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }

    pub fn bump(&self, release: &str, preid: &str) -> Result<Self> {
        if let Some(version) = Self::parse(release) {
            return Ok(version);
        }
        if !preid.is_empty() && !identifiers(preid, true) {
            bail!("invalid --preid");
        }
        let mut v = self.clone();
        v.build = None;
        let inc = |n: u64| n.checked_add(1).context("version number overflow");
        match release {
            "major" => {
                if v.minor != 0 || v.patch != 0 || v.pre.is_empty() {
                    v.major = inc(v.major)?;
                }
                v.minor = 0;
                v.patch = 0;
                v.pre.clear();
            }
            "minor" => {
                if v.patch != 0 || v.pre.is_empty() {
                    v.minor = inc(v.minor)?;
                }
                v.patch = 0;
                v.pre.clear();
            }
            "patch" => {
                if v.pre.is_empty() {
                    v.patch = inc(v.patch)?;
                }
                v.pre.clear();
            }
            "premajor" => {
                v.major = inc(v.major)?;
                v.minor = 0;
                v.patch = 0;
                v.pre.clear();
            }
            "preminor" => {
                v.minor = inc(v.minor)?;
                v.patch = 0;
                v.pre.clear();
            }
            "prepatch" => {
                v.patch = inc(v.patch)?;
                v.pre.clear();
            }
            "prerelease" => {
                if v.pre.is_empty() {
                    v.patch = inc(v.patch)?;
                }
            }
            _ => bail!("invalid version or release type: {release}"),
        }
        if release.starts_with("pre") {
            let prefix: Vec<String> = if preid.is_empty() {
                Vec::new()
            } else {
                preid.split('.').map(str::to_owned).collect()
            };
            if !prefix.is_empty() && (!v.pre.starts_with(&prefix) || v.pre.len() == prefix.len()) {
                v.pre = prefix;
                v.pre.push("0".into());
            } else if let Some(index) = v
                .pre
                .iter()
                .rposition(|p| p.bytes().all(|c| c.is_ascii_digit()))
            {
                let old: u64 = v.pre[index].parse().context("prerelease number overflow")?;
                v.pre[index] = inc(old)?.to_string();
            } else {
                v.pre.push("0".into());
            }
        }
        Ok(v)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if !self.pre.is_empty() {
            write!(f, "-{}", self.pre.join("."))?;
        }
        if let Some(build) = &self.build {
            write!(f, "+{build}")?;
        }
        Ok(())
    }
}

// npm's common range syntax, without introducing a second parser dependency.
// Non-registry specs (file:, git URLs, aliases, tags) return None.
pub fn satisfies(version: &str, range: &str) -> Option<bool> {
    let version = Version::parse(version)?;
    let mut result = false;
    for group in range.split("||") {
        result |= satisfies_group(&version, group.trim())?;
    }
    Some(result)
}

/// Ranges are unions of comparator intervals. An inhabited intersection has a
/// witness at a lower endpoint or its immediate stable/prerelease successor.
pub fn intersects(left: &str, right: &str) -> Option<bool> {
    satisfies("0.0.0", left)?;
    satisfies("0.0.0", right)?;
    let mut candidates = vec![Version::parse("0.0.0")?];
    for range in [left, right] {
        for text in range.split_whitespace().filter(|s| *s != "||" && *s != "-") {
            let text = text.trim_start_matches(['>', '<', '=', '^', '~']);
            if text.is_empty() {
                continue;
            }
            let Some((base, count)) = partial(text) else {
                continue;
            };
            candidates.push(base.clone());
            if !base.pre.is_empty() {
                let mut next = base.clone();
                next.pre.push("0".into());
                candidates.push(next);
                let mut stable = base.clone();
                stable.pre.clear();
                candidates.push(stable);
            }
            if count < 3 {
                if let Some(next) = upper(base.clone(), count) {
                    candidates.push(next);
                }
            } else if let Some(patch) = base.patch.checked_add(1) {
                let mut next = base;
                next.patch = patch;
                next.pre.clear();
                candidates.push(next);
            }
        }
    }
    Some(candidates.iter().any(|candidate| {
        let value = candidate.to_string();
        satisfies(&value, left) == Some(true) && satisfies(&value, right) == Some(true)
    }))
}

fn partial(value: &str) -> Option<(Version, usize)> {
    if let Some(version) = Version::parse(value) {
        return Some((version, 3));
    }
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts: Vec<_> = value.split('.').collect();
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
        Version {
            major: numbers[0],
            minor: numbers[1],
            patch: numbers[2],
            pre: Vec::new(),
            build: None,
        },
        count,
    ))
}

fn upper(mut version: Version, count: usize) -> Option<Version> {
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
    version.pre.clear();
    version.build = None;
    Some(version)
}

fn satisfies_group(version: &Version, group: &str) -> Option<bool> {
    if let Some((low, high)) = group.split_once(" - ") {
        let (low, _) = partial(low.trim())?;
        let (high, count) = partial(high.trim())?;
        let high_ok = if count == 3 {
            version.compare(&high) != Ordering::Greater
        } else if count == 0 {
            true
        } else {
            version.compare(&upper(high.clone(), count)?) == Ordering::Less
        };
        return Some(
            version.compare(&low) != Ordering::Less
                && high_ok
                && (version.pre.is_empty()
                    || (!low.pre.is_empty() && low.core() == version.core())
                    || (!high.pre.is_empty() && high.core() == version.core())),
        );
    }
    let tokens: Vec<_> = group.split_whitespace().collect();
    let mut result = true;
    let mut pre_allowed = version.pre.is_empty();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let (op, mut text) = [">=", "<=", ">", "<", "^", "~", "="]
            .into_iter()
            .find_map(|op| token.strip_prefix(op).map(|rest| (op, rest)))
            .unwrap_or(("", token));
        if text.is_empty() && !op.is_empty() {
            index += 1;
            text = *tokens.get(index)?;
        }
        let (low, count) = partial(text)?;
        pre_allowed |= !low.pre.is_empty() && low.core() == version.core();
        let order = version.compare(&low);
        result &= match op {
            "^" | "~" => {
                let limit = if count == 0 {
                    None
                } else if op == "~" {
                    upper(low.clone(), count.min(2))
                } else {
                    let position = if low.major > 0 || count == 1 {
                        1
                    } else if low.minor > 0 || count == 2 {
                        2
                    } else {
                        3
                    };
                    if position == 3 {
                        let mut limit = low.clone();
                        limit.patch = limit.patch.checked_add(1)?;
                        limit.pre.clear();
                        Some(limit)
                    } else {
                        upper(low.clone(), position)
                    }
                };
                order != Ordering::Less
                    && limit.is_none_or(|limit| version.compare(&limit) == Ordering::Less)
            }
            ">=" => count == 0 || order != Ordering::Less,
            ">" => {
                if count == 0 {
                    false
                } else if count < 3 {
                    version.compare(&upper(low, count)?) != Ordering::Less
                } else {
                    order == Ordering::Greater
                }
            }
            "<=" => {
                if count == 0 {
                    true
                } else if count < 3 {
                    version.compare(&upper(low, count)?) == Ordering::Less
                } else {
                    order != Ordering::Greater
                }
            }
            "<" => count != 0 && order == Ordering::Less,
            _ => {
                count == 0
                    || (order != Ordering::Less
                        && if count < 3 {
                            version.compare(&upper(low, count)?) == Ordering::Less
                        } else {
                            order == Ordering::Equal
                        })
            }
        };
        index += 1;
    }
    Some(result && pre_allowed)
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
            ("2.0.0", ">=1 <3", true),
            ("1.9.0", "1.x", true),
            ("2.0.0", "1.x", false),
            ("2.5.0", "1 || 2", true),
            ("2.9.0", "1.2 - 2", true),
            ("1.2.3-beta.1", "*", false),
            ("1.2.3-beta.2", ">=1.2.3-beta.1 <2", true),
        ] {
            assert_eq!(satisfies(v, range), Some(expected), "{v} {range}");
        }
        assert_eq!(satisfies("1.0.0", "file:../x"), None);
    }
}
