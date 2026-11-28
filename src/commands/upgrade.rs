//! Update the installed tnpm executable from official GitHub releases.
use std::{cmp::Ordering, io::Write, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};

use crate::{
    cli::Options,
    json::Value,
    semver::Version,
    util::style::progress::{Phase, Progress},
};

const API: &str = "https://api.github.com/repos/taisan11/tnpm/releases/latest";
const DOWNLOAD_BASE: &str = "https://github.com/taisan11/tnpm/releases/download/";
const MAX_METADATA: usize = 2 * 1024 * 1024;
const MAX_BINARY: usize = 256 * 1024 * 1024;

fn asset_name() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("tnpm-x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("tnpm-aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("tnpm-x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("tnpm-aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("tnpm-x86_64-pc-windows-msvc.exe"),
        (os, arch) => bail!("no official tnpm release for {os}/{arch}"),
    }
}

struct Release {
    tag: String,
    digest: String,
}

fn select_release(metadata: &str, asset: &str, current: &str) -> Result<Option<Release>> {
    let json = Value::parse(metadata).context("invalid GitHub release metadata")?;
    let tag = json.str("tag_name").context("release has no tag_name")?;
    ensure!(tag.starts_with('v'), "release tag must start with v");
    let version = Version::parse(tag).context("release tag is not a valid version")?;
    let current = Version::parse(current).context("current version is invalid")?;
    if version.compare(&current) != Ordering::Greater {
        return Ok(None);
    }
    let Some(Value::Array(assets)) = json.get("assets") else {
        bail!("release has no assets");
    };
    let mut matches = assets.iter().filter(|item| item.str("name") == Some(asset));
    let item = matches
        .next()
        .with_context(|| format!("release has no asset {asset}"))?;
    ensure!(
        matches.next().is_none(),
        "release has duplicate asset {asset}"
    );
    let digest = item
        .str("digest")
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("release asset has no SHA-256 digest")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid release SHA-256 digest"
    );
    Ok(Some(Release {
        tag: tag.to_owned(),
        digest: digest.to_ascii_lowercase(),
    }))
}

async fn get_limited(client: &reqwest::Client, url: reqwest::Url, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        bail!("release response exceeds size limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= limit - bytes.len(),
            "release response exceeds size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn download_update(
    client: &reqwest::Client,
    api: &str,
    base: &str,
    asset: &str,
    current: &str,
    progress: &Progress,
) -> Result<Option<(String, Vec<u8>)>> {
    progress.show(Phase::CheckingForUpdates);
    let metadata = get_limited(client, api.parse()?, MAX_METADATA)
        .await
        .context("could not fetch latest GitHub release")?;
    let metadata = std::str::from_utf8(&metadata).context("release metadata is not UTF-8")?;
    let Some(release) = select_release(metadata, asset, current)? else {
        return Ok(None);
    };
    let mut url = reqwest::Url::parse(base)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("invalid release download URL"))?
        .pop_if_empty()
        .push(&release.tag)
        .push(asset);
    progress.show(Phase::Downloading);
    let bytes = get_limited(client, url, MAX_BINARY)
        .await
        .context("could not download release binary")?;
    progress.show(Phase::Verifying);
    let actual = const_hex::encode(Sha256::digest(&bytes));
    ensure!(
        actual == release.digest,
        "release binary SHA-256 mismatch; current executable was not changed"
    );
    Ok(Some((release.tag, bytes)))
}

pub async fn run(options: &Options) -> Result<()> {
    let asset = asset_name()?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("tnpm/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()?;
    run_update(
        &client,
        API,
        DOWNLOAD_BASE,
        asset,
        env!("CARGO_PKG_VERSION"),
        options.json,
        replace_executable,
    )
    .await
}

async fn run_update(
    client: &reqwest::Client,
    api: &str,
    base: &str,
    asset: &str,
    current: &str,
    json: bool,
    replace: impl FnOnce(&[u8]) -> Result<()>,
) -> Result<()> {
    let progress = Progress::new(true);
    let tag = if let Some((tag, bytes)) =
        download_update(client, api, base, asset, current, &progress).await?
    {
        progress.show(Phase::Replacing);
        replace(&bytes)?;
        Some(tag)
    } else {
        None
    };
    drop(progress);
    if json {
        let version = tag
            .as_deref()
            .map_or(current, |tag| tag.strip_prefix('v').unwrap_or(tag));
        println!(
            "{}",
            nojson::json(|f| f.object(|f| {
                f.member("updated", tag.is_some())?;
                f.member("version", version)
            }))
        );
    } else if let Some(tag) = tag {
        println!("Updated tnpm to {tag}.");
    } else {
        println!("tnpm is already up to date (v{current}).");
    }
    Ok(())
}

fn replace_executable(bytes: &[u8]) -> Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new().context("could not create temporary update file")?;
    temporary
        .write_all(bytes)
        .context("could not write update")?;
    temporary.flush()?;
    let temporary = temporary.into_temp_path();
    // self-replace uses an atomic rename on Unix; Windows moves the locked executable
    // aside and starts a cleanup helper which waits for this process to exit.
    self_replace::self_replace(&temporary)
        .context("could not replace tnpm executable; check installation directory permissions")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::Command,
        thread,
    };

    #[test]
    fn selects_only_newer_versions_with_unique_valid_digest() {
        let digest = "a".repeat(64);
        let json = format!(
            r#"{{"tag_name":"v1.2.3","assets":[{{"name":"binary","digest":"sha256:{digest}"}}]}}"#
        );
        assert!(select_release(&json, "binary", "1.2.3").unwrap().is_none());
        assert!(select_release(&json, "binary", "2.0.0").unwrap().is_none());
        assert_eq!(
            select_release(&json, "binary", "1.2.2")
                .unwrap()
                .unwrap()
                .tag,
            "v1.2.3"
        );
        assert!(select_release(&json, "missing", "1.0.0").is_err());
        assert!(select_release(&json.replace(&digest, "bad"), "binary", "1.0.0").is_err());
        assert!(select_release(&json.replace("v1.2.3", "v../x"), "binary", "1.0.0").is_err());
        let duplicate = format!(
            r#"{{"tag_name":"v1.2.3","assets":[{{"name":"binary","digest":"sha256:{digest}"}},{{"name":"binary","digest":"sha256:{digest}"}}]}}"#
        );
        assert!(select_release(&duplicate, "binary", "1.0.0").is_err());
    }

    fn fixture_server(
        tag: &'static str,
        bad_digest: bool,
        request_count: usize,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let mut paths = Vec::new();
            for stream in listener.incoming().take(request_count) {
                let mut stream = stream.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while stream.read_exact(&mut byte).is_ok() {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                let body = if path == "/latest" {
                    let digest = if bad_digest {
                        "0".repeat(64)
                    } else {
                        const_hex::encode(Sha256::digest(b"new executable"))
                    };
                    format!(r#"{{"tag_name":"{tag}","assets":[{{"name":"binary","digest":"sha256:{digest}","browser_download_url":"https://evil.invalid/ignored"}}]}}"#).into_bytes()
                } else {
                    b"new executable".to_vec()
                };
                paths.push(path);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
            paths
        });
        (base, handle)
    }

    #[test]
    fn downloads_official_asset_and_rejects_modified_binary() {
        for bad_digest in [false, true] {
            let (base, server) = fixture_server("v1.2.3", bad_digest, 2);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let result = rt.block_on(download_update(
                &reqwest::Client::new(),
                &format!("{base}latest"),
                &format!("{base}download/"),
                "binary",
                "1.0.0",
                &Progress::new(false),
            ));
            if bad_digest {
                assert!(result.unwrap_err().to_string().contains("SHA-256 mismatch"));
            } else {
                let (tag, bytes) = result.unwrap().unwrap();
                assert_eq!(tag, "v1.2.3");
                assert_eq!(bytes, b"new executable");
            }
            assert_eq!(
                server.join().unwrap(),
                ["/latest", "/download/v1.2.3/binary"]
            );
        }
    }

    #[test]
    fn reports_upgrade_progress_on_stderr() {
        const BASE: &str = "TNPM_UPGRADE_TEST_BASE";
        const JSON: &str = "TNPM_UPGRADE_TEST_JSON";
        if let Ok(base) = std::env::var(BASE) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(run_update(
                &reqwest::Client::builder().no_proxy().build().unwrap(),
                &format!("{base}latest"),
                &format!("{base}download/"),
                "binary",
                "1.0.0",
                std::env::var_os(JSON).is_some(),
                |bytes| {
                    assert_eq!(bytes, b"new executable");
                    Ok(())
                },
            ))
            .unwrap();
            return;
        }

        for updated in [true, false] {
            for json in [false, true] {
                let (base, server) = if updated {
                    fixture_server("v1.2.3", false, 2)
                } else {
                    fixture_server("v1.0.0", false, 1)
                };
                let mut child = Command::new(std::env::current_exe().unwrap());
                child
                    .args([
                        "--exact",
                        "commands::upgrade::tests::reports_upgrade_progress_on_stderr",
                        "--nocapture",
                    ])
                    .env(BASE, base)
                    .env_remove(JSON);
                if json {
                    child.env(JSON, "1");
                }
                let output = child.output().unwrap();
                let stderr = String::from_utf8(output.stderr).unwrap();
                assert!(output.status.success(), "{stderr}");
                assert_eq!(
                    stderr,
                    if updated {
                        "Checking for updates\nDownloading\nVerifying\nReplacing\n"
                    } else {
                        "Checking for updates\n"
                    }
                );
                let stdout = String::from_utf8(output.stdout).unwrap();
                let result = match (updated, json) {
                    (true, false) => "Updated tnpm to v1.2.3.",
                    (false, false) => "tnpm is already up to date (v1.0.0).",
                    (true, true) => r#"{"updated":true,"version":"1.2.3"}"#,
                    (false, true) => r#"{"updated":false,"version":"1.0.0"}"#,
                };
                assert_eq!(
                    stdout
                        .lines()
                        .filter(|line| {
                            !line.is_empty()
                                && !line.starts_with("running ")
                                && !line.starts_with("test ")
                        })
                        .collect::<Vec<_>>(),
                    [result],
                    "{stdout:?}"
                );
                let paths = server.join().unwrap();
                assert_eq!(paths.len(), if updated { 2 } else { 1 });
            }
        }
    }

    #[test]
    fn replaces_a_running_executable_on_this_platform() {
        const CHILD: &str = "TNPM_UPGRADE_TEST_CHILD";
        if let Some(replacement) = std::env::var_os(CHILD) {
            self_replace::self_replace(replacement).unwrap();
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let executable = temp
            .path()
            .join(format!("runner{}", std::env::consts::EXE_SUFFIX));
        let replacement = temp
            .path()
            .join(format!("replacement{}", std::env::consts::EXE_SUFFIX));
        let mut bytes = fs::read(std::env::current_exe().unwrap()).unwrap();
        fs::write(&executable, &bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        bytes.extend_from_slice(b"tnpm upgrade replacement test");
        fs::write(&replacement, &bytes).unwrap();
        let status = Command::new(&executable)
            .args([
                "--exact",
                "commands::upgrade::tests::replaces_a_running_executable_on_this_platform",
            ])
            .env(CHILD, &replacement)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(fs::read(executable).unwrap(), bytes);
    }
}
