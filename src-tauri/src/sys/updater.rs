//! User-triggered yt-dlp updates from the official GitHub releases, verified against the
//! release's SHA2-256SUMS before anything is executed or activated.
//!
//! Updated copies live in Rabbit's user data folder as `yt-dlp-<version>.exe` (versioned names
//! never collide with a copy that is still running). `active.json` names the active copy; deleting
//! it falls back to the yt-dlp bundled with Rabbit.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::engine::http::Counters;
use crate::error::{AppError, AppResult};
use crate::sys::process::{command, spawn_contained};

/// Official standalone release asset for the running platform.
pub fn asset_name() -> &'static str {
    if cfg!(windows) { "yt-dlp.exe" }
    else if cfg!(target_arch = "aarch64") { "yt-dlp_linux_aarch64" }
    else { "yt-dlp_linux" }
}

pub fn installed_filename(tag: &str) -> String {
    super::tools::executable_name(&format!("yt-dlp-{tag}"))
}

fn valid_filename(name: &str) -> bool {
    let Some(tag) = name.strip_prefix("yt-dlp-").and_then(|n| n.strip_suffix(std::env::consts::EXE_SUFFIX)) else { return false };
    valid_tag(tag)
}

const MAX_BINARY: u64 = 150 * 1024 * 1024;
const MAX_SUMS: usize = 64 * 1024;
const MANIFEST: &str = "active.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Nightly,
}

impl Channel {
    pub fn repo(self) -> &'static str {
        match self {
            Channel::Stable => "yt-dlp/yt-dlp",
            Channel::Nightly => "yt-dlp/yt-dlp-nightly-builds",
        }
    }
}

/// Where releases come from. Production uses GitHub; tests point this at a local server.
#[derive(Debug, Clone)]
pub struct Source {
    pub api_base: String,
    pub download_base: String,
}

impl Default for Source {
    fn default() -> Self {
        Source { api_base: "https://api.github.com".into(), download_base: "https://github.com".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub channel: Channel,
    pub installed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub channel: Channel,
    pub current: Option<String>,
    pub latest: String,
    pub published_at: Option<String>,
    pub available: bool,
}

/// Release tags are dates, optionally with a build suffix: 2026.10.21 or 2026.10.21.233146.
pub fn valid_tag(tag: &str) -> bool {
    let parts: Vec<&str> = tag.split('.').collect();
    (3..=4).contains(&parts.len())
        && parts.iter().all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_digit()))
        && parts[0].len() == 4
}

fn version_key(v: &str) -> Vec<u64> {
    v.trim().split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// True when `candidate` is strictly newer than `current` (unknown current → any valid release is newer).
pub fn is_newer(candidate: &str, current: Option<&str>) -> bool {
    match current {
        None => true,
        Some(c) => version_key(candidate) > version_key(c),
    }
}

/// Finds the hex digest for `name` in a `sha256sum`-style file.
pub fn parse_sums(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut it = line.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The updated copy recorded in `dir`, if its file still exists.
pub fn active(dir: &Path) -> Option<(Installed, PathBuf)> {
    let m: Installed = serde_json::from_slice(&std::fs::read(dir.join(MANIFEST)).ok()?).ok()?;
    // The manifest may only name a file inside this folder.
    if m.file.contains(['/', '\\']) || !valid_filename(&m.file) {
        return None;
    }
    let path = dir.join(&m.file);
    super::tools::is_executable(&path).then_some((m, path))
}

/// Stops using the updated copy; the bundled yt-dlp takes over and updated copies are deleted
/// (any still running are left for a later cleanup).
pub fn revert(dir: &Path) -> AppResult<()> {
    match std::fs::remove_file(dir.join(MANIFEST)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    remove_stale(dir, "");
    Ok(())
}

fn user_agent() -> String {
    format!("Rabbit/{} (yt-dlp updater)", env!("CARGO_PKG_VERSION"))
}

pub async fn latest(client: &reqwest::Client, src: &Source, channel: Channel) -> AppResult<(String, Option<String>)> {
    #[derive(Deserialize)]
    struct Release {
        tag_name: String,
        published_at: Option<String>,
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        draft: bool,
    }
    let url = format!("{}/repos/{}/releases/latest", src.api_base, channel.repo());
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, user_agent())
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    if resp.status() == reqwest::StatusCode::FORBIDDEN || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::Network("GitHub is rate-limiting update checks right now. Try again in a few minutes.".into()));
    }
    if !resp.status().is_success() {
        return Err(AppError::Http(resp.status().as_u16()));
    }
    let body = resp.bytes().await?;
    let r: Release = serde_json::from_slice(&body).map_err(|e| AppError::Network(format!("unexpected release data: {e}")))?;
    if r.draft || (channel == Channel::Stable && r.prerelease) || !valid_tag(&r.tag_name) {
        return Err(AppError::Verification(format!("the latest release tag '{}' isn't in the expected form", r.tag_name.chars().take(40).collect::<String>())));
    }
    Ok((r.tag_name, r.published_at))
}

async fn fetch_sums(client: &reqwest::Client, url: &str) -> AppResult<String> {
    let resp = client.get(url).header(reqwest::header::USER_AGENT, user_agent()).timeout(Duration::from_secs(30)).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(resp.status().as_u16()));
    }
    let bytes = resp.bytes().await?;
    if bytes.len() > MAX_SUMS {
        return Err(AppError::Verification("the checksum file is unexpectedly large".into()));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Runs `<exe> --version` (contained, no console window) and returns its trimmed output.
pub async fn run_version(exe: &Path) -> AppResult<String> {
    let proc = spawn_contained(command(exe, ["--ignore-config", "--version"])).map_err(|e| AppError::Tool(format!("couldn't start yt-dlp: {e}")))?;
    let killer = proc.killer();
    let out = tokio::time::timeout(Duration::from_secs(60), proc.child.wait_with_output()).await;
    match out {
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Ok(Ok(o)) => Err(AppError::Tool(format!("yt-dlp --version failed: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")))),
        Ok(Err(e)) => Err(AppError::Tool(e.to_string())),
        Err(_) => {
            killer.kill();
            Err(AppError::Tool("yt-dlp --version didn't finish in 60 s".into()))
        }
    }
}

/// Downloads `tag`, verifies its SHA-256 against the release's checksum file, checks that it
/// runs and reports the same version, then makes it the active copy.
pub async fn install(client: &reqwest::Client, src: &Source, channel: Channel, tag: &str, dir: &Path, progress: &Counters, cancel: &CancellationToken) -> AppResult<Installed> {
    if !valid_tag(tag) {
        return Err(AppError::Input("invalid release tag".into()));
    }
    // Asset URLs are built from fixed parts, never taken from API responses.
    let base = format!("{}/{}/releases/download/{}", src.download_base, channel.repo(), tag);
    let sums = fetch_sums(client, &format!("{base}/SHA2-256SUMS")).await.map_err(|e| match e {
        AppError::Http(404) => AppError::NotFound(format!("yt-dlp {tag} isn't published on the {channel:?} channel. Check for updates again.").replace("Stable", "stable").replace("Nightly", "nightly")),
        other => other,
    })?;
    let expected = parse_sums(&sums, asset_name()).ok_or_else(|| AppError::Verification(format!("the release's checksum file doesn't list {}", asset_name())))?;

    tokio::fs::create_dir_all(dir).await?;
    let file = installed_filename(tag);
    let final_path = dir.join(&file);
    let tmp = dir.join(format!("{file}.download"));

    let result = async {
        let resp = tokio::select! {
            r = client.get(format!("{base}/{}", asset_name())).header(reqwest::header::USER_AGENT, user_agent()).send() => r?,
            _ = cancel.cancelled() => return Err(AppError::Cancelled),
        };
        if !resp.status().is_success() {
            return Err(AppError::Http(resp.status().as_u16()));
        }
        let total = resp.content_length();
        if total.is_some_and(|t| t > MAX_BINARY) {
            return Err(AppError::Verification("the download is larger than any yt-dlp release".into()));
        }
        progress.total.store(total.unwrap_or(0), Ordering::Relaxed);
        progress.done.store(0, Ordering::Relaxed);

        let mut out = tokio::fs::File::create(&tmp).await?;
        let mut hasher = Sha256::new();
        let mut stream = resp.bytes_stream();
        let mut written = 0u64;
        loop {
            let next = tokio::select! {
                n = tokio::time::timeout(Duration::from_secs(30), stream.next()) => n,
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
            };
            let chunk = match next {
                Err(_) => return Err(AppError::Network("the download stalled for 30 s".into())),
                Ok(None) => break,
                Ok(Some(c)) => c?,
            };
            written += chunk.len() as u64;
            if written > MAX_BINARY {
                return Err(AppError::Verification("the download is larger than any yt-dlp release".into()));
            }
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            progress.done.store(written, Ordering::Relaxed);
        }
        out.flush().await?;
        out.sync_all().await?;
        drop(out);

        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected {
            return Err(AppError::Verification(format!("checksum mismatch for yt-dlp {tag} — expected {expected}, got {actual}. Nothing was installed")));
        }
        Ok(actual)
    }
    .await;

    let sha256 = match result {
        Ok(h) => h,
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e);
        }
    };

    // Verified: only now may it be moved into place and executed.
    if final_path.exists() {
        let _ = tokio::fs::remove_file(&final_path).await;
    }
    tokio::fs::rename(&tmp, &final_path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&final_path, std::fs::Permissions::from_mode(0o755)).await?;
    }
    let reported = match run_version(&final_path).await {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&final_path).await;
            return Err(e);
        }
    };
    if reported != tag {
        let _ = tokio::fs::remove_file(&final_path).await;
        return Err(AppError::Verification(format!("the downloaded yt-dlp reports version {reported}, expected {tag}")));
    }

    let installed = Installed { version: tag.to_string(), file, sha256, channel, installed_at: now_ms() };
    let manifest = dir.join(MANIFEST);
    let tmp_manifest = dir.join(format!("{MANIFEST}.tmp"));
    tokio::fs::write(&tmp_manifest, serde_json::to_vec_pretty(&installed).map_err(|e| AppError::Io(e.to_string()))?).await?;
    tokio::fs::rename(&tmp_manifest, &manifest).await?;
    remove_stale(dir, &installed.file);
    Ok(installed)
}

/// Deletes older updated copies. A copy still in use by a running yt-dlp is skipped and
/// removed on a later run.
fn remove_stale(dir: &Path, keep: &str) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if (valid_filename(&name) || name.strip_suffix(".download").is_some_and(valid_filename)) && name != keep {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert!(valid_tag("2026.10.21"));
        assert!(valid_tag("2026.10.21.233146"));
        assert!(!valid_tag("2026.10"));
        assert!(!valid_tag("v2026.10.21"));
        assert!(!valid_tag("2026.10.21/../../x"));
        assert!(!valid_tag("2026.10.21.1.2"));
        assert!(!valid_tag("26.10.21"));
    }

    #[test]
    fn version_order() {
        assert!(is_newer("2026.10.30", Some("2026.10.21")));
        assert!(is_newer("2026.10.21.233146", Some("2026.10.21")));
        assert!(!is_newer("2026.10.21", Some("2026.10.21")));
        assert!(!is_newer("2026.08.19", Some("2026.10.21")));
        assert!(is_newer("2026.11.01", Some("2026.10.21.233146")));
        assert!(is_newer("2026.10.21", None));
    }

    #[test]
    fn sums_parsing() {
        let text = "aa  yt-dlp\n33121d192d706e62f684a06f80918e3f3a8ba52ac3a65213b6951c39c7d9f555  yt-dlp.exe\nbb *yt-dlp_x86.exe\n";
        assert_eq!(parse_sums(text, "yt-dlp.exe").as_deref(), Some("33121d192d706e62f684a06f80918e3f3a8ba52ac3a65213b6951c39c7d9f555"));
        assert_eq!(parse_sums(text, "yt-dlp_x86.exe"), None, "short hashes are rejected");
        assert_eq!(parse_sums(text, "missing.exe"), None);
    }

    #[test]
    fn manifest_cannot_point_outside_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = installed_filename("2026.10.21");
        let path = dir.path().join(&file);
        std::fs::write(&path, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let good = Installed { version: "2026.10.21".into(), file, sha256: "0".repeat(64), channel: Channel::Stable, installed_at: 0 };
        std::fs::write(dir.path().join(MANIFEST), serde_json::to_vec(&good).unwrap()).unwrap();
        assert!(active(dir.path()).is_some());
        for bad in ["..\\evil.exe", "C:\\Windows\\notepad.exe", "yt-dlp-x.exe.bat", "other.exe"] {
            let m = Installed { file: bad.into(), ..good.clone() };
            std::fs::write(dir.path().join(MANIFEST), serde_json::to_vec(&m).unwrap()).unwrap();
            assert!(active(dir.path()).is_none(), "{bad}");
        }
        revert(dir.path()).unwrap();
        assert!(active(dir.path()).is_none());
        revert(dir.path()).unwrap(); // idempotent
    }
}
