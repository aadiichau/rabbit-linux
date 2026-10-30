//! yt-dlp updater against a local fake GitHub (API + release downloads) serving the real binary.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rabbit_lib::engine::http::Counters;
use rabbit_lib::engine::manager::Manager;
use rabbit_lib::error::AppError;
use rabbit_lib::store::{Settings, Store};
use rabbit_lib::sys::tools::Tools;
use rabbit_lib::sys::updater::{self, Channel, Source};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn real_ytdlp() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) { "binaries/yt-dlp-x86_64-pc-windows-msvc.exe" } else { "binaries/linux/yt-dlp" })
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Serves fixed bodies by path; unknown paths get 404.
async fn serve(routes: HashMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let routes = Arc::new(routes);
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let routes = routes.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut got = 0;
                while !buf[..got].windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = sock.read(&mut buf[got..]).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    got += n;
                }
                let req = String::from_utf8_lossy(&buf[..got]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                match routes.get(&path) {
                    Some(body) => {
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body).await;
                    }
                    None => {
                        let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    }
                }
            });
        }
    });
    format!("http://{addr}")
}

/// A fake release: `api_tag` is what the API reports, `tag` is where the files live.
fn release(api_tag: &str, tag: &str, exe: &[u8], sums_hash: &str) -> HashMap<String, Vec<u8>> {
    let mut r = HashMap::new();
    r.insert("/repos/yt-dlp/yt-dlp/releases/latest".into(), format!(r#"{{"tag_name":"{api_tag}","published_at":"2026-10-21T21:54:37Z","prerelease":false,"draft":false}}"#).into_bytes());
    r.insert(format!("/yt-dlp/yt-dlp/releases/download/{tag}/SHA2-256SUMS"), format!("{}  yt-dlp\n{sums_hash}  {}\n", "0".repeat(64), updater::asset_name()).into_bytes());
    r.insert(format!("/yt-dlp/yt-dlp/releases/download/{tag}/{}", updater::asset_name()), exe.to_vec());
    r
}

fn manager(dir: &Path, src: Source) -> Arc<Manager> {
    let store = Arc::new(Store::open(&dir.join("t.db")).unwrap());
    let mut tools = Tools::discover();
    tools.bundled_ytdlp = Some(real_ytdlp());
    tools.ytdlp = Some(real_ytdlp());
    let s = Settings { download_dir: dir.join("out"), max_concurrent: 1, max_segments: 4, notifications: false, ytdlp_channel: Channel::Stable, sessions: Default::default() };
    let m = Manager::new(store, s, tools).unwrap();
    m.set_update_source(src);
    m.set_update_dir(dir.join("bin"));
    m
}

fn exe_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn verified_install_switches_live_and_reverts() {
    let exe = std::fs::read(real_ytdlp()).unwrap();
    let base = serve(release("2026.10.21", "2026.10.21", &exe, &sha_hex(&exe))).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), Source { api_base: base.clone(), download_base: base });

    let before = m.ytdlp_status().await;
    assert_eq!(before.source, "bundled");
    assert_eq!(before.version.as_deref(), Some("2026.10.21"));

    // Same version on another channel? No — same channel and same version: nothing to do.
    let check = m.ytdlp_check(Channel::Stable).await.unwrap();
    assert_eq!(check.latest, "2026.10.21");
    assert!(!check.available, "the bundled copy is already this version");

    let after = m.ytdlp_install(Channel::Stable, &check.latest, None).await.unwrap();
    assert_eq!(after.source, "update");
    assert_eq!(after.version.as_deref(), Some("2026.10.21"));
    let active = m.tools().ytdlp.unwrap();
    assert_eq!(active, dir.path().join("bin").join(updater::installed_filename("2026.10.21")), "new operations use the updated copy");
    assert_eq!(after.update.unwrap().sha256, sha_hex(&exe));

    // A fresh manager (app restart) picks the update up from the manifest.
    let restarted = manager(dir.path(), Source::default());
    assert_eq!(restarted.ytdlp_status().await.source, "update");

    let reverted = m.ytdlp_use_bundled().await.unwrap();
    assert_eq!(reverted.source, "bundled");
    assert_eq!(m.tools().ytdlp.unwrap(), real_ytdlp());
    assert!(exe_files(&dir.path().join("bin")).iter().all(|f| !f.starts_with("yt-dlp-")), "updated copies are removed on revert");
}

#[tokio::test]
async fn tampered_checksum_installs_nothing() {
    let exe = std::fs::read(real_ytdlp()).unwrap();
    let wrong = sha_hex(b"not the binary");
    let base = serve(release("2026.10.21", "2026.10.21", &exe, &wrong)).await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), Source { api_base: base.clone(), download_base: base });

    let err = m.ytdlp_install(Channel::Stable, "2026.10.21", None).await.unwrap_err();
    assert!(matches!(err, AppError::Verification(_)), "{err}");
    assert!(err.to_string().contains("checksum mismatch"), "{err}");
    assert!(exe_files(&dir.path().join("bin")).is_empty(), "no binary or partial file may remain: {:?}", exe_files(&dir.path().join("bin")));
    assert_eq!(m.ytdlp_status().await.source, "bundled");
}

#[tokio::test]
async fn binary_reporting_another_version_is_rejected() {
    // Files published under 2026.10.30 whose binary actually reports 2026.10.21.
    let exe = std::fs::read(real_ytdlp()).unwrap();
    let base = serve(release("2026.10.30", "2026.10.30", &exe, &sha_hex(&exe))).await;
    let dir = tempfile::tempdir().unwrap();
    let src = Source { api_base: base.clone(), download_base: base };
    let counters = Counters::default();
    let err = updater::install(&reqwest::Client::new(), &src, Channel::Stable, "2026.10.30", &dir.path().join("bin"), &counters, &CancellationToken::new()).await.unwrap_err();
    assert!(err.to_string().contains("reports version 2026.10.21"), "{err}");
    assert!(updater::active(&dir.path().join("bin")).is_none());
    assert!(exe_files(&dir.path().join("bin")).iter().all(|f| !f.starts_with("yt-dlp-")));
}

/// Real GitHub: check both channels, install the nightly (verified) into a temp folder, then try
/// the Bilibili link that is broken in the bundled 2026.10.21.
///   cargo test --test updater live -- --ignored --nocapture
#[tokio::test]
#[ignore = "network"]
async fn live_github_check_and_nightly_install() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path(), Source::default());
    for ch in [Channel::Stable, Channel::Nightly] {
        let c = m.ytdlp_check(ch).await.unwrap();
        println!("{ch:?}: latest {} (current {:?}) available={}", c.latest, c.current, c.available);
    }
    let nightly = m.ytdlp_check(Channel::Nightly).await.unwrap();
    let t0 = std::time::Instant::now();
    let status = m.ytdlp_install(Channel::Nightly, &nightly.latest, None).await.unwrap();
    println!("installed {:?} from {:?} in {:.1} s, sha256 {}", status.version, status.source, t0.elapsed().as_secs_f64(), status.update.as_ref().unwrap().sha256);
    assert_eq!(status.version.as_deref(), Some(nightly.latest.as_str()));

    match m.analyze("https://www.bilibili.com/video/BV13x41117TL").await {
        Ok(info) => println!("bilibili with {}: OK — {:?}, {} options", nightly.latest, info.title, info.options.len()),
        Err(e) => println!("bilibili with {}: still failing — {e}", nightly.latest),
    }
}

#[tokio::test]
async fn malicious_tag_from_api_is_refused() {
    let base = serve(release("../../../evil", "x", b"", &"0".repeat(64))).await;
    let src = Source { api_base: base.clone(), download_base: base };
    let err = updater::latest(&reqwest::Client::new(), &src, Channel::Stable).await.unwrap_err();
    assert!(matches!(err, AppError::Verification(_)), "{err}");
    let err = updater::install(&reqwest::Client::new(), &src, Channel::Stable, "../../../evil", Path::new("."), &Counters::default(), &CancellationToken::new()).await.unwrap_err();
    assert!(matches!(err, AppError::Input(_)), "{err}");
}
