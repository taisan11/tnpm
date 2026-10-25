//! Versioned, indexed binary registry cache. No full JSON reparse is needed on a hit.
//!
//! v3 interns text in contiguous UTF-8 pools, keeps object entries in original
//! order with a separate lookup index, and persists a SemVer-sorted version index.
//! Body and HTTP policy have independent pools so 304 refreshes preserve the body.
use super::http_cache::Policy;
use crate::{json::Value, semver::Version};
use rustc_hash::FxHashMap;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    ops::Range,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use xxhash_rust::xxh3::Xxh3Default;

const MAGIC: &[u8; 8] = b"TNPMREG\0";
const FORMAT: u32 = 3;
const HEADER: usize = 88;
const MAX_SIZE: usize = 128 * 1024 * 1024;
const MAX_NODES: usize = 2_000_000;
const MAX_DEPTH: usize = 128;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub struct Cache {
    directory: PathBuf,
}

impl Cache {
    pub fn home() -> Option<Self> {
        dirs::home_dir().map(|home| Self {
            directory: home.join(".tnpm/cache"),
        })
    }

    fn path(&self, key: &str) -> PathBuf {
        let hash = format!("{:016x}", checksum(key.as_bytes()));
        self.directory
            .join("v3")
            .join(&hash[..2])
            .join(format!("{}.bin", &hash[2..]))
    }

    pub fn load(&self, key: &str) -> io::Result<Option<Document>> {
        let file = match File::open(self.path(key)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let size = usize::try_from(file.metadata()?.len()).map_err(|_| invalid())?;
        if size > MAX_SIZE {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(size);
        file.take(MAX_SIZE as u64 + 1).read_to_end(&mut bytes)?;
        Document::open(bytes, key)
    }

    pub fn store(&self, key: &str, value: &Value, policy: &Policy) -> io::Result<()> {
        if !policy.can_store() {
            return self.remove(key);
        }
        self.write(key, &encode(key, value, policy)?)
    }

    pub fn refresh(&self, key: &str, document: &Document, policy: &Policy) -> io::Result<()> {
        if !policy.can_store() {
            return self.remove(key);
        }
        // Preserve the indexed body verbatim: 304 must not decode/re-encode all versions.
        let mut writer = Encoder::new(document.bytes[..document.layout.http].to_vec());
        let value = policy.to_value();
        let http = writer.node(&value, 0)?;
        let http_pool = writer.flush_strings()?;
        let layout = Layout {
            http,
            http_pool,
            ..document.layout.clone()
        };
        finish(&mut writer, key.len(), &layout, policy)?;
        self.write(key, &writer.bytes)
    }

    pub fn remove(&self, key: &str) -> io::Result<()> {
        match fs::remove_file(self.path(key)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    fn write(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        let destination = self.path(key);
        let directory = destination.parent().ok_or_else(invalid)?;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let temporary = directory.join(format!(
            ".{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        let result = (|| {
            file.write_all(bytes)?;
            drop(file);
            fs::rename(&temporary, &destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

// Stable FNV-1a for filenames. XXH3-128 detects accidental file corruption;
// neither hash authenticates the private cache. Tarball SRI remains cryptographic.
// The full URL is also checked, so a hash collision can only cause a cache miss.
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

fn file_checksum(bytes: &[u8]) -> [u8; 16] {
    let mut hash = Xxh3Default::new();
    hash.update(&bytes[..40]);
    hash.update(&bytes[56..]);
    hash.digest128().to_le_bytes()
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid registry cache")
}

fn encode(key: &str, value: &Value, policy: &Policy) -> io::Result<Vec<u8>> {
    let mut writer = Encoder::new(vec![0; HEADER]);
    writer.bytes.extend_from_slice(key.as_bytes());
    let root = writer.node(value, 0)?;
    let (versions, all_versions_valid) = writer.version_index(value, root)?;
    let body_pool = writer.flush_strings()?;
    let policy_value = policy.to_value();
    let http = writer.node(&policy_value, 0)?;
    let http_pool = writer.flush_strings()?;
    let layout = Layout {
        root,
        http,
        body_pool,
        http_pool,
        versions,
        all_versions_valid,
    };
    finish(&mut writer, key.len(), &layout, policy)?;
    Ok(writer.bytes)
}

fn finish(
    writer: &mut Encoder<'_>,
    key_size: usize,
    layout: &Layout,
    policy: &Policy,
) -> io::Result<()> {
    let size = writer.bytes.len();
    if size > MAX_SIZE {
        return Err(invalid());
    }
    writer.bytes[..8].copy_from_slice(MAGIC);
    writer.u32(8, FORMAT as usize)?;
    writer.u32(12, layout.root)?;
    writer.bytes[16..24].copy_from_slice(&policy.response_seconds().to_le_bytes());
    writer.bytes[24..32].copy_from_slice(&policy.expires_seconds().to_le_bytes());
    writer.u32(32, size)?;
    writer.u32(36, key_size)?;
    writer.u32(56, layout.http)?;
    writer.u32(60, layout.body_pool.start)?;
    writer.u32(64, layout.body_pool.len())?;
    writer.u32(68, layout.http_pool.start)?;
    writer.u32(72, layout.http_pool.len())?;
    writer.u32(76, layout.versions.start)?;
    writer.u32(80, layout.versions.len() / 12)?;
    writer.u32(84, usize::from(layout.all_versions_valid))?;
    let hash = file_checksum(&writer.bytes);
    writer.bytes[40..56].copy_from_slice(&hash);
    Ok(())
}

#[derive(Clone)]
struct Layout {
    root: usize,
    http: usize,
    body_pool: Range<usize>,
    http_pool: Range<usize>,
    versions: Range<usize>,
    all_versions_valid: bool,
}

struct Encoder<'a> {
    bytes: Vec<u8>,
    nodes: usize,
    strings: FxHashMap<&'a str, usize>,
    pool: Vec<u8>,
    references: Vec<usize>,
}

impl<'a> Encoder<'a> {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            nodes: 0,
            strings: FxHashMap::default(),
            pool: Vec::new(),
            references: Vec::new(),
        }
    }

    fn string(&mut self, at: usize, text: &'a str) -> io::Result<()> {
        let offset = if let Some(&offset) = self.strings.get(text) {
            offset
        } else {
            let offset = self.pool.len();
            if offset.saturating_add(text.len()) > MAX_SIZE {
                return Err(invalid());
            }
            self.pool.extend_from_slice(text.as_bytes());
            self.strings.insert(text, offset);
            offset
        };
        self.u32(at, offset)?;
        self.u32(at + 4, text.len())?;
        self.references.push(at);
        Ok(())
    }

    fn flush_strings(&mut self) -> io::Result<Range<usize>> {
        let start = self.bytes.len();
        for i in 0..self.references.len() {
            let at = self.references[i];
            let relative = u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap()) as usize;
            self.u32(at, start.checked_add(relative).ok_or_else(invalid)?)?;
        }
        self.bytes.append(&mut self.pool);
        self.strings.clear();
        self.references.clear();
        Ok(start..self.bytes.len())
    }

    fn version_index(&mut self, value: &'a Value, root: usize) -> io::Result<(Range<usize>, bool)> {
        let start = self.bytes.len();
        let Some(index) = value.members().iter().position(|(k, _)| k == "versions") else {
            return Ok((start..start, true));
        };
        let versions = &value.members()[index].1;
        if !matches!(versions, Value::Object(_)) {
            return Ok((start..start, true));
        }
        let at = root + 5 + index * 12 + 8;
        let object = u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap()) as usize;
        let mut sorted: Vec<_> = versions
            .members()
            .iter()
            .enumerate()
            .map(|(i, (name, _))| (i, name.as_str(), Version::parse(name)))
            .collect();
        let all_valid = sorted.iter().all(|(_, _, version)| version.is_some());
        sorted.sort_by_key(|(_, name, _)| *name);
        sorted.sort_by(|(_, _, a), (_, _, b)| match (a, b) {
            (Some(a), Some(b)) => a.compare(b),
            _ => std::cmp::Ordering::Equal,
        });
        let mut first = FxHashMap::with_capacity_and_hasher(sorted.len(), Default::default());
        for (index, (name, _)) in versions.members().iter().enumerate() {
            first.entry(name.as_str()).or_insert(index);
        }
        self.bytes.resize(start + sorted.len() * 12, 0);
        for (i, (_, name, _)) in sorted.into_iter().enumerate() {
            let entry = start + i * 12;
            self.string(entry, name)?;
            let at = object + 5 + first[name] * 12 + 8;
            let child = u32::from_le_bytes(self.bytes[at..at + 4].try_into().unwrap()) as usize;
            self.u32(entry + 8, child)?;
        }
        Ok((start..self.bytes.len(), all_valid))
    }

    fn u32(&mut self, offset: usize, value: usize) -> io::Result<()> {
        let value = u32::try_from(value).map_err(|_| invalid())?;
        self.bytes
            .get_mut(offset..offset + 4)
            .ok_or_else(invalid)?
            .copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn node(&mut self, value: &'a Value, depth: usize) -> io::Result<usize> {
        self.nodes += 1;
        if depth > MAX_DEPTH || self.nodes > MAX_NODES || self.bytes.len() > MAX_SIZE {
            return Err(invalid());
        }
        let offset = self.bytes.len();
        match value {
            Value::Null => self.bytes.push(0),
            Value::Bool(false) => self.bytes.push(1),
            Value::Bool(true) => self.bytes.push(2),
            Value::Number(text) | Value::String(text) => {
                self.bytes.push(if matches!(value, Value::Number(_)) {
                    3
                } else {
                    4
                });
                self.bytes.extend_from_slice(&[0; 8]);
                self.string(offset + 1, text)?;
            }
            Value::Array(values) => {
                self.bytes.push(5);
                self.bytes.resize(offset + 5 + values.len() * 4, 0);
                self.u32(offset + 1, values.len())?;
                for (index, value) in values.iter().enumerate() {
                    let child = self.node(value, depth + 1)?;
                    self.u32(offset + 5 + index * 4, child)?;
                }
            }
            Value::Object(members) => {
                self.bytes.push(6);
                self.bytes.resize(offset + 5 + members.len() * 16, 0);
                self.u32(offset + 1, members.len())?;
                let mut sorted: Vec<_> = members.iter().enumerate().collect();
                sorted.sort_by(|(_, (a, _)), (_, (b, _))| a.cmp(b));
                for (index, (original, _)) in sorted.into_iter().enumerate() {
                    self.u32(offset + 5 + members.len() * 12 + index * 4, original)?;
                }
                for (index, (key, value)) in members.iter().enumerate() {
                    let entry = offset + 5 + index * 12;
                    self.string(entry, key)?;
                    let child = self.node(value, depth + 1)?;
                    self.u32(entry + 8, child)?;
                }
            }
        }
        Ok(offset)
    }
}

pub struct Document {
    bytes: Vec<u8>,
    start: usize,
    layout: Layout,
    policy: Option<Policy>,
}

impl Document {
    fn open(bytes: Vec<u8>, key: &str) -> io::Result<Option<Self>> {
        if bytes.len() < HEADER || bytes.len() > MAX_SIZE || &bytes[..8] != MAGIC {
            return Err(invalid());
        }
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        if u32_at(8) != FORMAT as usize || u32_at(32) != bytes.len() || u32_at(84) > 1 {
            return Err(invalid());
        }
        let start = HEADER.checked_add(u32_at(36)).ok_or_else(invalid)?;
        if bytes.get(HEADER..start) != Some(key.as_bytes()) {
            return Ok(None);
        }
        if file_checksum(&bytes).as_slice() != &bytes[40..56] {
            return Err(invalid());
        }
        let root = u32_at(12);
        let http = u32_at(56);
        let range = |offset, length| -> io::Result<Range<usize>> {
            let start = u32_at(offset);
            let end = start.checked_add(u32_at(length)).ok_or_else(invalid)?;
            if end > bytes.len() {
                return Err(invalid());
            }
            Ok(start..end)
        };
        let body_pool = range(60, 64)?;
        let http_pool = range(68, 72)?;
        let version_start = u32_at(76);
        let count = u32_at(80);
        let all_versions_valid = u32_at(84) == 1;
        let version_end = version_start
            .checked_add(count.checked_mul(12).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        if root < start
            || root >= body_pool.start
            || body_pool.end != http
            || http >= http_pool.start
            || http_pool.end != bytes.len()
            || count > MAX_NODES
            || version_start < root
            || version_end > body_pool.start
        {
            return Err(invalid());
        }
        // Validate each shared pool once. Individual reads still check UTF-8
        // boundaries, but do not rescan the same dependency text for every version.
        simdutf8::basic::from_utf8(&bytes[body_pool.clone()]).map_err(|_| invalid())?;
        simdutf8::basic::from_utf8(&bytes[http_pool.clone()]).map_err(|_| invalid())?;
        let mut document = Self {
            bytes,
            start,
            layout: Layout {
                root,
                http,
                body_pool,
                http_pool,
                versions: version_start..version_end,
                all_versions_valid,
            },
            policy: None,
        };
        if document.root().tag()? != 6 {
            return Err(invalid());
        }
        let policy = Policy::from_value(
            &Node {
                document: &document,
                offset: http,
            }
            .value()?,
        )
        .ok_or_else(invalid)?;
        if !policy.can_store() {
            return Err(invalid());
        }
        document.policy = Some(policy);
        Ok(Some(document))
    }

    pub fn policy(&self) -> &Policy {
        self.policy
            .as_ref()
            .expect("cache policy was validated while opening the document")
    }

    pub fn root(&self) -> Node<'_> {
        Node {
            document: self,
            offset: self.layout.root,
        }
    }

    pub fn version_count(&self) -> usize {
        self.layout.versions.len() / 12
    }

    pub fn all_versions_valid(&self) -> bool {
        self.layout.all_versions_valid
    }

    pub fn version(&self, index: usize) -> io::Result<(&str, Node<'_>)> {
        if index >= self.version_count() {
            return Err(invalid());
        }
        let at = self.layout.versions.start + index * 12;
        self.root().entry_at(at)
    }
}

#[derive(Clone, Copy)]
pub struct Node<'a> {
    document: &'a Document,
    offset: usize,
}

impl<'a> Node<'a> {
    fn bytes(self, offset: usize, length: usize) -> io::Result<&'a [u8]> {
        let layout = &self.document.layout;
        let range = if self.offset < layout.http {
            self.document.start..layout.body_pool.start
        } else {
            layout.http..layout.http_pool.start
        };
        if self.offset < range.start
            || self.offset >= range.end
            || offset < range.start
            || offset.checked_add(length).is_none_or(|end| end > range.end)
        {
            return Err(invalid());
        }
        self.document
            .bytes
            .get(offset..offset.checked_add(length).ok_or_else(invalid)?)
            .ok_or_else(invalid)
    }

    fn u32(self, offset: usize) -> io::Result<usize> {
        Ok(u32::from_le_bytes(self.bytes(offset, 4)?.try_into().map_err(|_| invalid())?) as usize)
    }

    fn tag(self) -> io::Result<u8> {
        Ok(self.bytes(self.offset, 1)?[0])
    }

    fn string_at(self, offset: usize, length: usize) -> io::Result<&'a str> {
        let layout = &self.document.layout;
        let pool = if self.offset < layout.http {
            &layout.body_pool
        } else {
            &layout.http_pool
        };
        let start = offset.checked_sub(pool.start).ok_or_else(invalid)?;
        let end = start.checked_add(length).ok_or_else(invalid)?;
        // SAFETY: Document::open validated this complete, immutable pool as UTF-8.
        let text = unsafe { std::str::from_utf8_unchecked(&self.document.bytes[pool.clone()]) };
        text.get(start..end).ok_or_else(invalid)
    }

    fn table(self, tag: u8, entry_size: usize) -> io::Result<usize> {
        if self.tag()? != tag {
            return Err(invalid());
        }
        let count = self.u32(self.offset + 1)?;
        if count > MAX_NODES {
            return Err(invalid());
        }
        self.bytes(
            self.offset + 5,
            count.checked_mul(entry_size).ok_or_else(invalid)?,
        )?;
        Ok(count)
    }

    fn entry_at(self, at: usize) -> io::Result<(&'a str, Self)> {
        let key = self.string_at(self.u32(at)?, self.u32(at + 4)?)?;
        let node = Self {
            document: self.document,
            offset: self.u32(at + 8)?,
        };
        Ok((key, node))
    }

    fn entry(self, index: usize) -> io::Result<(&'a str, Self)> {
        self.entry_at(self.offset + 5 + index * 12)
    }

    fn sorted_entry(self, index: usize, count: usize) -> io::Result<(&'a str, Self)> {
        let original = self.u32(self.offset + 5 + count * 12 + index * 4)?;
        if original >= count {
            return Err(invalid());
        }
        self.entry(original)
    }

    pub fn member(self, key: &str) -> io::Result<Option<Self>> {
        let count = self.table(6, 16)?;
        let (mut low, mut high) = (0, count);
        while low < high {
            let middle = low + (high - low) / 2;
            if self.sorted_entry(middle, count)?.0 < key {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        if low < count {
            let (name, value) = self.sorted_entry(low, count)?;
            if name == key {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn string(self) -> io::Result<&'a str> {
        if self.tag()? != 4 {
            return Err(invalid());
        }
        self.string_at(self.u32(self.offset + 1)?, self.u32(self.offset + 5)?)
    }

    pub fn value(self) -> io::Result<Value> {
        let mut budget = MAX_NODES;
        self.decode(0, &mut budget)
    }

    pub fn object_except(self, exclude: &str) -> io::Result<Value> {
        let mut budget = MAX_NODES;
        self.object(0, &mut budget, Some(exclude))
    }

    fn object(self, depth: usize, budget: &mut usize, exclude: Option<&str>) -> io::Result<Value> {
        let count = self.table(6, 16)?;
        if count > *budget {
            return Err(invalid());
        }
        let mut members = Vec::with_capacity(count);
        for index in 0..count {
            let (key, value) = self.entry(index)?;
            if Some(key) != exclude {
                members.push((key.into(), value.decode(depth + 1, budget)?));
            }
        }
        Ok(Value::Object(members))
    }

    fn decode(self, depth: usize, budget: &mut usize) -> io::Result<Value> {
        if depth > MAX_DEPTH || *budget == 0 {
            return Err(invalid());
        }
        *budget -= 1;
        Ok(match self.tag()? {
            0 => Value::Null,
            1 => Value::Bool(false),
            2 => Value::Bool(true),
            tag @ (3 | 4) => {
                let text =
                    self.string_at(self.u32(self.offset + 1)?, self.u32(self.offset + 5)?)?;
                if tag == 4 {
                    Value::from(text)
                } else {
                    let value = Value::parse(text).map_err(|_| invalid())?;
                    if !matches!(value, Value::Number(_)) {
                        return Err(invalid());
                    }
                    value
                }
            }
            5 => {
                let count = self.table(5, 4)?;
                if count > *budget {
                    return Err(invalid());
                }
                let mut values = Vec::with_capacity(count);
                for index in 0..count {
                    let child = Self {
                        document: self.document,
                        offset: self.u32(self.offset + 5 + index * 4)?,
                    };
                    values.push(child.decode(depth + 1, budget)?);
                }
                Value::Array(values)
            }
            6 => self.object(depth, budget, None)?,
            _ => return Err(invalid()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};
    const KEY: &str = "https://registry.npmjs.org/@scope%2Fpkg";

    fn encoded(value: &Value) -> Vec<u8> {
        let now = UNIX_EPOCH + Duration::from_secs(100);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CACHE_CONTROL,
            reqwest::header::HeaderValue::from_static("max-age=300"),
        );
        encode(
            KEY,
            value,
            &Policy::new(
                &headers,
                &super::super::http_cache::request_headers(),
                now,
                now,
            ),
        )
        .unwrap()
    }

    #[test]
    fn binary_roundtrip_preserves_json_order_and_lexemes() {
        let value = Value::parse(r#"{"z":9007199254740993,"a":[null,false,true,"日本語",1.20e3],"versions":{"2.0.0":{"version":"2.0.0"}}}"#).unwrap();
        let document = Document::open(encoded(&value), KEY).unwrap().unwrap();
        assert_eq!(document.root().value().unwrap(), value);
        assert_eq!(
            document
                .root()
                .member("z")
                .unwrap()
                .unwrap()
                .value()
                .unwrap(),
            Value::Number("9007199254740993".into())
        );
        assert!(document.root().member("missing").unwrap().is_none());
        assert!(
            document
                .root()
                .object_except("versions")
                .unwrap()
                .get("versions")
                .is_none()
        );
    }

    #[test]
    fn invalid_utf8_is_rejected_even_when_the_cache_checksum_matches() {
        let mut value = Value::object();
        value.set("description", Value::from("日本語🦀".repeat(16).as_str()));
        let mut bytes = encoded(&value);
        let document = Document::open(bytes.clone(), KEY).unwrap().unwrap();
        let node = document.root().member("description").unwrap().unwrap();
        let offset = node.u32(node.offset + 1).unwrap();
        bytes[offset + 100] = 0xff;
        rehash(&mut bytes);
        assert_eq!(
            Document::open(bytes, KEY).err().unwrap().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn expiry_identity_format_and_checksum_are_checked() {
        let value = Value::parse(r#"{"versions":{}}"#).unwrap();
        let bytes = encoded(&value);
        let document = Document::open(bytes.clone(), KEY).unwrap().unwrap();
        // Keep expired records readable so their validators can be used.
        for (seconds, fresh) in [(99, false), (399, true), (400, false)] {
            assert_eq!(
                document
                    .policy()
                    .is_fresh(UNIX_EPOCH + Duration::from_secs(seconds)),
                fresh
            );
        }
        assert!(
            Document::open(bytes.clone(), "https://other.test/pkg")
                .unwrap()
                .is_none()
        );
        for index in [0, 8, 12, 32, HEADER + KEY.len()] {
            let mut corrupt = bytes.clone();
            corrupt[index] ^= 0xff;
            assert!(Document::open(corrupt, KEY).is_err(), "offset {index}");
        }
        for length in [0, 7, HEADER - 1, HEADER, bytes.len() - 1] {
            assert!(Document::open(bytes[..length].to_vec(), KEY).is_err());
        }
    }

    #[test]
    fn index_resolves_a_version_without_decoding_other_manifests() {
        let value = Value::parse(r#"{"versions":{"1.0.0":{"name":"old"},"2.0.0":{"name":"new"}}}"#)
            .unwrap();
        let mut bytes = encoded(&value);
        let document = Document::open(bytes.clone(), KEY).unwrap().unwrap();
        let offset = document
            .root()
            .member("versions")
            .unwrap()
            .unwrap()
            .member("1.0.0")
            .unwrap()
            .unwrap()
            .offset;
        // Even an unreadable sibling does not need to be visited during indexed lookup.
        bytes[offset] = 0xff;
        rehash(&mut bytes);
        let document = Document::open(bytes, KEY).unwrap().unwrap();
        assert_eq!(
            document
                .root()
                .member("versions")
                .unwrap()
                .unwrap()
                .member("2.0.0")
                .unwrap()
                .unwrap()
                .value()
                .unwrap()
                .str("name"),
            Some("new")
        );
        assert!(document.root().value().is_err());
    }

    #[test]
    fn repeated_text_is_shared_and_original_object_order_is_preserved() {
        let text = "共通の説明🦀".repeat(8);
        let mut versions = Value::object();
        for i in 0..2 {
            let mut version = Value::object();
            version.set("description", Value::from(text.as_str()));
            version.set("license", Value::from("MIT"));
            versions.set(&format!("1.0.{i}"), version);
        }
        let mut value = Value::object();
        value.set("versions", versions);
        let bytes = encoded(&value);
        let document = Document::open(bytes, KEY).unwrap().unwrap();
        assert_eq!(document.root().value().unwrap(), value);
        let versions = document.root().member("versions").unwrap().unwrap();
        let description = |name| {
            versions
                .member(name)
                .unwrap()
                .unwrap()
                .member("description")
                .unwrap()
                .unwrap()
                .string()
                .unwrap()
        };
        assert!(std::ptr::eq(
            description("1.0.0").as_ptr(),
            description("1.0.1").as_ptr()
        ));
        assert_eq!(document.version_count(), 2);
        assert_eq!(document.version(1).unwrap().0, "1.0.1");

        let duplicate = Value::parse(r#"{"z":"last","a":"first","a":"second","versions":{"1.0.0":{"name":"first"},"1.0.0":{"name":"second"}}}"#).unwrap();
        let document = Document::open(encoded(&duplicate), KEY).unwrap().unwrap();
        assert_eq!(document.root().value().unwrap(), duplicate);
        assert_eq!(
            document
                .root()
                .member("a")
                .unwrap()
                .unwrap()
                .string()
                .unwrap(),
            "first"
        );
        assert_eq!(
            document.version(1).unwrap().1.value().unwrap().str("name"),
            Some("first")
        );
    }

    fn rehash(bytes: &mut [u8]) {
        let hash = file_checksum(bytes);
        bytes[40..56].copy_from_slice(&hash);
    }

    #[test]
    fn malformed_pools_indexes_boundaries_and_recursive_nodes_are_rejected() {
        let value =
            Value::parse(r#"{"description":"日本語🦀","versions":{"1.0.0":{"version":"1.0.0"}}}"#)
                .unwrap();
        let bytes = encoded(&value);
        for at in [8, 12, 56, 60, 64, 68, 72, 76, 80, 84] {
            let mut corrupt = bytes.clone();
            corrupt[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            rehash(&mut corrupt);
            assert!(Document::open(corrupt, KEY).is_err(), "header offset {at}");
        }
        let mut old = bytes.clone();
        old[8..12].copy_from_slice(&2_u32.to_le_bytes());
        rehash(&mut old);
        assert!(Document::open(old, KEY).is_err());

        let document = Document::open(bytes.clone(), KEY).unwrap().unwrap();
        let description = document.root().member("description").unwrap().unwrap();
        let text = description.u32(description.offset + 1).unwrap();
        let mut corrupt = bytes.clone();
        // The pool is valid UTF-8, but a reference must not start inside a code point.
        corrupt[description.offset + 1..description.offset + 5]
            .copy_from_slice(&((text + 1) as u32).to_le_bytes());
        rehash(&mut corrupt);
        let corrupt = Document::open(corrupt, KEY).unwrap().unwrap();
        assert!(corrupt.root().value().is_err());

        let root = document.root().offset;
        let count = value.members().len();
        let mut corrupt = bytes.clone();
        let index = root + 5 + count * 12;
        corrupt[index..index + 4].copy_from_slice(&(count as u32).to_le_bytes());
        rehash(&mut corrupt);
        let corrupt = Document::open(corrupt, KEY).unwrap().unwrap();
        assert!(corrupt.root().member("description").is_err());

        let mut corrupt = bytes;
        let child = root + 5 + 8;
        corrupt[child..child + 4].copy_from_slice(&(root as u32).to_le_bytes());
        rehash(&mut corrupt);
        let corrupt = Document::open(corrupt, KEY).unwrap().unwrap();
        assert!(corrupt.root().value().is_err()); // A cyclic offset must hit the depth limit.
    }

    #[test]
    fn binary_queries_match_json_for_tags_ranges_ties_duplicates_and_invalid_versions() {
        use crate::registry::{Query, resolve_binary, resolve_json};
        // SemVer syntax itself is tested in semver.rs; exercise each codec-specific
        // boundary with only the selectors that distinguish that boundary.
        for (source, selectors) in [
            (
                r#"{"name":"fixture","dist-tags":{"latest":"2.0.0","next":"3.0.0-beta.2","missing":"9.0.0"},"versions":{"2.0.0":{"version":"2.0.0"},"1.0.0+z":{"version":"1.0.0+z"},"1.0.0+a":{"version":"1.0.0+a"},"0.9.0":{"version":"0.9.0"},"3.0.0-beta.2":{"version":"3.0.0-beta.2"},"3.0.0-beta.11":{"version":"3.0.0-beta.11"},"1.2.0":{"version":"1.2.0"},"1.0.0":{"version":"1.0.0"}}}"#,
                &[
                    "latest",
                    "next",
                    "missing",
                    "unknown",
                    "v1.0.0",
                    "^1",
                    "=1.0.0",
                    "^3.0.0-beta.2",
                ][..],
            ),
            (
                r#"{"versions":{"1.0.0":{"version":"1.0.0"},"1.0.0":{"version":"duplicate"},"2.0.0":{"version":"2.0.0"}}}"#,
                &["1.0.0", "*"][..],
            ),
            (
                r#"{"versions":{"1.0.0":{"version":"1.0.0"},"not-semver":{"version":"invalid"},"2.0.0":{"version":"2.0.0"}}}"#,
                &["1.0.0", "^1"][..],
            ),
            (
                r#"{"dist-tags":{"latest":"2.0.0"},"versions":{"1.0.0":false,"2.0.0":{"version":"2.0.0"}}}"#,
                &["latest", "*"][..],
            ),
            (r#"{"versions":{}}"#, &["*", "unknown"][..]),
        ] {
            let value = Value::parse(source).unwrap();
            let document = Document::open(encoded(&value), KEY).unwrap().unwrap();
            let projected = vec!["versions".into()];
            for &selector in selectors {
                for query in [
                    Query::from(selector),
                    Query::versions(selector, false),
                    Query::versions(selector, true),
                    Query::view(selector, &projected),
                ] {
                    let convert = |result: anyhow::Result<crate::registry::Package>| {
                        result
                            .map(|p| (p.metadata, p.versions))
                            .map_err(|e| e.to_string())
                    };
                    assert_eq!(
                        convert(resolve_binary(&document, query)),
                        convert(resolve_json(&value, query)),
                        "selector {selector}, source {source}"
                    );
                }
            }
        }
    }

    struct TestCache(Cache);

    impl TestCache {
        fn new() -> Self {
            Self(Cache {
                directory: std::env::temp_dir().join(format!(
                    "tnpm-registry-v3-{}-{}",
                    std::process::id(),
                    crate::fs::unique()
                )),
            })
        }
    }

    impl Drop for TestCache {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.directory);
        }
    }

    #[test]
    fn revalidation_replaces_only_the_http_section_and_no_store_removes_the_entry() {
        let cache = TestCache::new();
        let value = Value::parse(r#"{"name":"fixture","dist-tags":{"latest":"1.0.0"},"versions":{"1.0.0":{"version":"1.0.0","license":"MIT"}}}"#).unwrap();
        let first = Document::open(encoded(&value), KEY).unwrap().unwrap();
        cache.0.store(KEY, &value, first.policy()).unwrap();
        let original = cache.0.load(KEY).unwrap().unwrap();
        let body = &original.bytes[HEADER..original.layout.http];
        let request_headers = super::super::http_cache::request_headers();
        for now in [500, 600] {
            let time = UNIX_EPOCH + Duration::from_secs(now);
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::CACHE_CONTROL,
                reqwest::header::HeaderValue::from_static("max-age=600"),
            );
            headers.insert(
                reqwest::header::ETAG,
                reqwest::header::HeaderValue::from_static("\"fixture\""),
            );
            let old = cache.0.load(KEY).unwrap().unwrap();
            let policy = old
                .policy()
                .revalidated(&headers, &request_headers, time, time);
            cache.0.refresh(KEY, &old, &policy).unwrap();
            let updated = cache.0.load(KEY).unwrap().unwrap();
            assert_eq!(&updated.bytes[HEADER..updated.layout.http], body);
            assert_eq!(updated.root().value().unwrap(), value);
            assert!(updated.policy().is_fresh(time + Duration::from_secs(599)));
            assert_eq!(
                updated.policy().conditional_headers()[reqwest::header::IF_NONE_MATCH],
                "\"fixture\""
            );
        }
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CACHE_CONTROL,
            reqwest::header::HeaderValue::from_static("no-store"),
        );
        let now = UNIX_EPOCH + Duration::from_secs(800);
        let policy = Policy::new(&headers, &request_headers, now, now);
        let old = cache.0.load(KEY).unwrap().unwrap();
        cache.0.refresh(KEY, &old, &policy).unwrap();
        assert!(cache.0.load(KEY).unwrap().is_none());
        assert!(cache.0.directory.join("v3").is_dir());
    }
}
