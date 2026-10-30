//! Live tests for Rabbit's own Bilibili, RedNote and Kuaishou extractors. Ignored by default (need network):
//!   cargo test --test native_platforms -- --ignored --nocapture --test-threads=1
//! The links are public posts. RedNote video notes only open through a share link, and share links
//! identify the person who shared them, so none is committed: set RABBIT_TEST_REDNOTE_VIDEO to one.
//! Output goes to a temp folder; the user's settings and library are untouched.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rabbit_lib::engine::manager::{Manager, TaskInfo, TaskState};
use rabbit_lib::engine::post;
use rabbit_lib::extract::model::{MediaInfo, OptionGroup};
use rabbit_lib::store::{Settings, Store};
use rabbit_lib::sys::tools::Tools;
use tokio_util::sync::CancellationToken;

const BILI: &str = "https://www.bilibili.com/video/BV1efJk6aEdA?t=1.6";
const BILI_MULTIPART: &str = "https://www.bilibili.com/video/BV1bK411W797?p=2";
const BILI_LONG: &str = "https://www.bilibili.com/video/BV13x41117TL"; // 9 min, large enough to pause mid-transfer
const KS_SHORT: &str = "https://v.kuaishou.com/5fkjkb"; // DLBunny's documented example
const KS_DIRECT: &str = "https://www.kuaishou.com/short-video/3xib8y4uv3b4zu2"; // the same post as KS_SHORT

/// A RedNote video share link from the environment (never committed: it identifies the sharer).
fn rednote_video() -> Option<String> {
    std::env::var("RABBIT_TEST_REDNOTE_VIDEO").ok().filter(|s| !s.is_empty())
}

fn tools() -> Tools {
    let mut t = Tools::discover();
    t.ytdlp = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) { "binaries/yt-dlp-x86_64-pc-windows-msvc.exe" } else { "binaries/linux/yt-dlp" }));
    t
}

fn manager(dir: &Path) -> Arc<Manager> {
    let store = Arc::new(Store::open(&dir.join("test.db")).unwrap());
    let settings = Settings { download_dir: dir.join("out"), max_concurrent: 2, max_segments: 8, notifications: false, ytdlp_channel: Default::default(), sessions: Default::default() };
    Manager::new(store, settings, tools()).unwrap()
}

async fn wait_for(m: &Manager, id: &str, limit: Duration, cond: impl Fn(&TaskInfo) -> bool) -> TaskInfo {
    let start = Instant::now();
    loop {
        if let Some(t) = m.list().into_iter().find(|t| t.id == id) {
            if cond(&t) {
                return t;
            }
            assert!(start.elapsed() < limit, "timed out waiting; task is {:?} ({} bytes) {:?}", t.state, t.done, t.error);
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn finished(t: &TaskInfo) -> bool {
    !matches!(t.state, TaskState::Queued | TaskState::Downloading | TaskState::Processing | TaskState::Verifying)
}

/// Downloads one option and returns the finished file, asserting the task completed.
async fn download(m: &Arc<Manager>, info: &MediaInfo, option_id: &str) -> std::path::PathBuf {
    let id = m.enqueue(&info.analysis_id, option_id).unwrap().id;
    let t = wait_for(m, &id, Duration::from_secs(300), finished).await;
    assert_eq!(t.state, TaskState::Completed, "{option_id}: {:?}", t.error);
    let p = std::path::PathBuf::from(t.file_path.unwrap());
    assert!(std::fs::metadata(&p).unwrap().len() > 0);
    p
}

fn assert_metadata(info: &MediaInfo, extractor: &str) {
    println!("{} | {} | {:?} | {:?}s | {} options | {} ms", info.platform_name, info.title, info.uploader, info.duration, info.options.len(), info.elapsed_ms);
    assert!(info.extractor.contains(extractor), "extractor {}", info.extractor);
    assert!(!info.title.is_empty() && info.title != "Untitled");
    assert!(info.uploader.is_some(), "uploader");
    assert!(info.duration.is_some_and(|d| d > 1.0), "duration");
    assert!(info.thumbnail.as_deref().is_some_and(|t| t.starts_with("data:image/")), "thumbnail fetched");
    assert!(info.upload_date.is_some(), "upload date");
    assert!(info.options.iter().any(|o| o.recommended), "a recommended option");
}

#[tokio::test]
#[ignore = "network"]
async fn bilibili_video_audio_and_cover() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let tools = tools();
    let info = m.analyze(BILI).await.expect("analyze");
    assert_metadata(&info, "Bilibili (Rabbit");
    assert_eq!(info.id, "BV1efJk6aEdA");

    let rec = info.options.iter().find(|o| o.recommended).unwrap();
    assert_eq!((rec.group, rec.container.as_str()), (OptionGroup::Video, "mp4"));
    let video = download(&m, &info, &rec.id).await;
    let p = post::probe(&tools, &video, &CancellationToken::new()).await.unwrap();
    assert!(p.video >= 1 && p.audio >= 1 && p.duration.is_some_and(|d| (d - 61.0).abs() < 2.0), "{p:?}");

    let m4a = info.options.iter().find(|o| o.group == OptionGroup::Audio && o.container == "m4a").expect("original audio");
    let audio = download(&m, &info, &m4a.id).await;
    let p = post::probe(&tools, &audio, &CancellationToken::new()).await.unwrap();
    assert!(p.video == 0 && p.audio == 1, "{p:?}");

    let mp3 = info.options.iter().find(|o| o.container == "mp3").expect("mp3 conversion");
    let mp3 = download(&m, &info, &mp3.id).await;
    assert_eq!(mp3.extension().unwrap(), "mp3");

    let cover = download(&m, &info, "cover").await;
    assert_eq!(cover.extension().unwrap(), "jpg");
}

#[tokio::test]
#[ignore = "network"]
async fn bilibili_multipart_part_and_bad_part() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let info = m.analyze(BILI_MULTIPART).await.expect("analyze");
    assert_metadata(&info, "Bilibili (Rabbit");
    assert!(info.title.contains(" - P2"), "part in title: {}", info.title);
    assert_eq!(info.id, "BV1bK411W797_p2");
    assert!(info.source_url.ends_with("?p=2"), "re-analysis keeps the part: {}", info.source_url);

    let e = m.analyze("https://www.bilibili.com/video/BV1bK411W797?p=999").await.unwrap_err();
    println!("p=999 → {e}");
    assert_eq!(e.kind(), "input");
}

#[tokio::test]
#[ignore = "network"]
async fn bilibili_resume_after_restart_reanalyzes_natively() {
    let dir = tempfile::tempdir().unwrap();
    let a = manager(dir.path());
    let info = a.analyze(BILI_LONG).await.expect("analyze");
    let opt = info.options.iter().filter(|o| o.group == OptionGroup::Video).max_by_key(|o| o.size.unwrap_or(0)).unwrap();
    println!("option {} ≈{:?} bytes", opt.id, opt.size);
    let id = a.enqueue(&info.analysis_id, &opt.id).unwrap().id;
    wait_for(&a, &id, Duration::from_secs(120), |t| t.done > 4_000_000).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let b = manager(dir.path());
    a.pause(&id).unwrap();
    wait_for(&a, &id, Duration::from_secs(30), |t| t.state == TaskState::Paused).await;
    let restored = b.list().into_iter().find(|t| t.id == id).unwrap();
    println!("restored with {} bytes saved", restored.done);
    assert!(restored.done > 1_000_000);

    // Stream links aren't stored, so this resume must re-analyze through the native extractor.
    b.resume(&id).unwrap();
    let done = wait_for(&b, &id, Duration::from_secs(600), finished).await;
    assert_eq!(done.state, TaskState::Completed, "{:?}", done.error);
    let p = post::probe(&tools(), Path::new(done.file_path.as_deref().unwrap()), &CancellationToken::new()).await.unwrap();
    assert!(p.video >= 1 && p.audio >= 1 && p.duration.is_some_and(|d| d > 500.0), "{p:?}");
}

#[tokio::test]
#[ignore = "network"]
async fn rednote_video_note_from_share_link() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let tools = tools();
    let Some(link) = rednote_video() else {
        println!("RABBIT_TEST_REDNOTE_VIDEO not set — skipped");
        return;
    };
    let info = m.analyze(&link).await.expect("analyze");
    assert_metadata(&info, "RedNote (Rabbit");
    assert_eq!(info.kind, rabbit_lib::extract::model::MediaKind::Video);
    assert!(info.source_url.contains("xsec_token="), "re-analysis keeps the share token");

    let rec = info.options.iter().find(|o| o.recommended).unwrap();
    let video = download(&m, &info, &rec.id).await;
    let p = post::probe(&tools, &video, &CancellationToken::new()).await.unwrap();
    assert!(p.video >= 1 && p.audio >= 1, "{p:?}");

    let mp3 = info.options.iter().find(|o| o.container == "mp3").expect("mp3");
    download(&m, &info, &mp3.id).await;
    let cover = download(&m, &info, "cover").await;
    assert_eq!(cover.extension().unwrap(), "jpg");
}

#[tokio::test]
#[ignore = "network"]
async fn kuaishou_short_and_direct_links() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let tools = tools();
    let short = m.analyze(KS_SHORT).await.expect("analyze short link");
    assert_metadata(&short, "Kuaishou (Rabbit");
    let direct = m.analyze(KS_DIRECT).await.expect("analyze direct link");
    assert_eq!(short.id, direct.id, "both links name the same post");
    assert_eq!(short.title, direct.title);

    let rec = short.options.iter().find(|o| o.recommended).unwrap();
    let video = download(&m, &short, &rec.id).await;
    let p = post::probe(&tools, &video, &CancellationToken::new()).await.unwrap();
    assert!(p.video >= 1 && p.audio >= 1 && p.duration.is_some_and(|d| (d - 50.3).abs() < 1.5), "{p:?}");

    let mp3 = direct.options.iter().find(|o| o.container == "mp3").expect("mp3");
    download(&m, &direct, &mp3.id).await;
}

/// Analysis latency (docs/BENCHMARKS.md): three runs per link in one process, so run 1 includes
/// connection setup (and Bilibili's visitor session) and runs 2–3 reuse them.
#[tokio::test]
#[ignore = "network"]
async fn analysis_latency() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let links: Vec<String> = [Some(BILI.to_string()), rednote_video(), Some(KS_SHORT.to_string()), Some(KS_DIRECT.to_string())].into_iter().flatten().collect();
    for url in &links {
        let url = url.as_str();
        let mut ms = Vec::new();
        for _ in 0..3 {
            let t = Instant::now();
            m.analyze(url).await.expect(url);
            ms.push(t.elapsed().as_millis());
        }
        println!("{url:<55} {ms:?} ms");
    }
    // The yt-dlp route these links used before (Bilibili fails upstream; timing until the error).
    let t = Instant::now();
    let r = rabbit_lib::extract::ytdlp::dump_json(&tools(), BILI, &CancellationToken::new()).await;
    println!("yt-dlp -J {BILI}: {} ms, ok={}", t.elapsed().as_millis(), r.is_ok());
}

#[tokio::test]
#[ignore = "network"]
async fn known_failures_have_specific_messages() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let cases = [
        // An expired b23.tv code: Bilibili answers -404 instead of redirecting.
        ("https://b23.tv/a9P3uZO", "extractor", "short link"),
        // A well-formed BV id that doesn't exist.
        ("https://www.bilibili.com/video/BV1A1111111A", "extractor", "doesn't exist"),
        // RedNote note links without their share token are refused by RedNote.
        ("https://www.xiaohongshu.com/explore/672451ee000000001a034bc6", "input", "share link"),
        // DLBunny's own RedNote example: the share code now redirects to the home page.
        ("http://xhslink.com/a/RVjdnIaGmsi2", "extractor", "expired"),
        ("https://www.kuaishou.com/profile/3xabcdefg", "unsupported", "doesn't lead to a video"),
    ];
    for (url, kind, needle) in cases {
        let e = m.analyze(url).await.expect_err(url);
        println!("{url}\n  → [{}] {e}", e.kind());
        assert_eq!(e.kind(), kind, "{url}: {e}");
        assert!(e.to_string().contains(needle), "{url}: {e}");
    }
}

// ---------- signed-in sessions (opt-in) ----------
// These use YOUR browser session and never run unless you point them at it:
//   $env:RABBIT_SESSION_SOURCE = "<Firefox profile folder>"   (or a cookies.txt file)
//   $env:RABBIT_DOUYIN_URL = "https://www.douyin.com/video/<id>"   (optional; a public video)
//   cargo test --test native_platforms session -- --ignored --nocapture --test-threads=1
// Sign in to bilibili.com / douyin.com in that Firefox profile first. The tests read cookies only
// for the site under test and never print them.

fn session_manager(dir: &Path, bilibili: bool, douyin: bool) -> Option<Arc<Manager>> {
    use rabbit_lib::sys::cookies::{CookieSource, Sessions};
    let raw = std::env::var("RABBIT_SESSION_SOURCE").ok()?;
    let path = std::path::PathBuf::from(raw);
    let source = if path.is_dir() { CookieSource::FirefoxProfile { path } } else { CookieSource::NetscapeFile { path } };
    let store = Arc::new(Store::open(&dir.join("test.db")).unwrap());
    let sessions = Sessions { enabled: true, source: Some(source), bilibili, douyin };
    let settings = Settings { download_dir: dir.join("out"), max_concurrent: 2, max_segments: 8, notifications: false, ytdlp_channel: Default::default(), sessions };
    Some(Manager::new(store, settings, tools()).unwrap())
}

#[tokio::test]
#[ignore = "needs your signed-in Firefox profile (RABBIT_SESSION_SOURCE)"]
async fn session_bilibili_hd() {
    use rabbit_lib::sys::cookies::Site;
    let dir = tempfile::tempdir().unwrap();
    let Some(m) = session_manager(dir.path(), true, false) else {
        println!("RABBIT_SESSION_SOURCE not set — skipped");
        return;
    };
    let status = m.session_check(Site::Bilibili).await.expect("session check");
    println!("Bilibili session: {} {:?} — {}", status.state, status.account, status.detail);
    assert_eq!(status.state, "signedIn", "sign in to bilibili.com in that Firefox profile first");

    let info = m.analyze(BILI).await.expect("analyze");
    println!("session: {:?}", info.session);
    assert!(info.session.as_deref().is_some_and(|s| s.starts_with("your Bilibili session")), "{:?}", info.session);
    let best = info.options.iter().filter(|o| o.group == OptionGroup::Video).max_by_key(|o| o.height.unwrap_or(0)).unwrap();
    println!("best option: {} {}", best.label, best.codec);
    assert!(best.height.unwrap_or(0).min(best.width.unwrap_or(u32::MAX)) > 480, "a signed-in session offers more than the guest 480p");
    let file = download(&m, &info, &best.id).await;
    let p = post::probe(&tools(), &file, &CancellationToken::new()).await.unwrap();
    println!("file: {:?}x{:?} {:?}+{:?}", p.width, p.height, p.vcodec, p.acodec);
    assert!(p.short_side().is_some_and(|s| s >= 720) && p.audio >= 1, "{p:?}");
}

#[tokio::test]
#[ignore = "needs your signed-in Firefox profile (RABBIT_SESSION_SOURCE)"]
async fn session_douyin_video() {
    use rabbit_lib::sys::cookies::Site;
    let dir = tempfile::tempdir().unwrap();
    let Some(m) = session_manager(dir.path(), false, true) else {
        println!("RABBIT_SESSION_SOURCE not set — skipped");
        return;
    };
    let status = m.session_check(Site::Douyin).await.expect("session check");
    println!("Douyin session: {} — {}", status.state, status.detail);
    assert_eq!(status.state, "signedIn", "sign in to douyin.com in that Firefox profile first");

    let url = std::env::var("RABBIT_DOUYIN_URL").unwrap_or_else(|_| "https://www.douyin.com/video/6961737553342991651".into());
    let info = m.analyze(&url).await.expect("analyze with your Douyin session");
    assert_eq!(info.session.as_deref(), Some("your Douyin session"));
    let rec = info.options.iter().find(|o| o.recommended).unwrap();
    println!("{} | {} {}", info.title, rec.label, rec.codec);
    let file = download(&m, &info, &rec.id).await;
    let p = post::probe(&tools(), &file, &CancellationToken::new()).await.unwrap();
    println!("file: {:?}x{:?} {:?}+{:?}", p.width, p.height, p.vcodec, p.acodec);
    assert!(p.video >= 1, "{p:?}");
    // The temporary cookies file is gone once analysis returns.
    let leftovers = std::fs::read_dir(std::env::temp_dir().join("rabbit-sessions")).map(|rd| rd.count()).unwrap_or(0);
    assert_eq!(leftovers, 0, "temporary session files must be deleted");
}

#[tokio::test]
#[ignore = "network"]
async fn sessions_off_by_default_change_nothing() {
    // Default settings: Douyin explains how to opt in; Bilibili stays at guest qualities without a note.
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let e = m.analyze("https://www.douyin.com/video/6961737553342991651").await.unwrap_err();
    assert!(e.to_string().contains("Signed-in sites") || e.detail().is_some_and(|d| d.contains("Signed-in sites")), "{e} / {:?}", e.detail());
    let info = m.analyze(BILI).await.unwrap();
    assert_eq!(info.session, None);
}
#[tokio::test]
#[ignore = "network"]
async fn session_fallbacks_with_a_signed_out_cookie_file() {
    // A cookies.txt with made-up (signed-out) values: no real account involved.
    use rabbit_lib::sys::cookies::{CookieSource, Sessions, Site};
    let dir = tempfile::tempdir().unwrap();
    let jar = dir.path().join("cookies.txt");
    std::fs::write(&jar, ".bilibili.com\tTRUE\t/\tTRUE\t4102444800\tSESSDATA\tnot-a-real-session\n.douyin.com\tTRUE\t/\tFALSE\t4102444800\tttwid\tanonymous\n").unwrap();
    let store = Arc::new(Store::open(&dir.path().join("test.db")).unwrap());
    let sessions = Sessions { enabled: true, source: Some(CookieSource::NetscapeFile { path: jar }), bilibili: true, douyin: true };
    let settings = Settings { download_dir: dir.path().join("out"), max_concurrent: 2, max_segments: 8, notifications: false, ytdlp_channel: Default::default(), sessions };
    let m = Manager::new(store, settings, tools()).unwrap();

    let b = m.session_check(Site::Bilibili).await.unwrap();
    println!("bilibili check: {} — {}", b.state, b.detail);
    assert_eq!(b.state, "signedOut");
    let d = m.session_check(Site::Douyin).await.unwrap();
    println!("douyin check: {} — {}", d.state, d.detail);
    assert_eq!(d.state, "signedOut", "cookies but no login cookie");

    // Bilibili falls back to guest access and says why.
    let info = m.analyze(BILI).await.unwrap();
    println!("session note: {:?}", info.session);
    assert!(info.session.as_deref().is_some_and(|s| s.starts_with("guest qualities")), "{:?}", info.session);
    assert!(info.options.iter().filter(|o| o.group == OptionGroup::Video).all(|o| o.height.unwrap_or(0) <= 480));

    // Douyin with a session that isn't signed in: the error says to sign in again; the temp file is gone.
    let e = m.analyze("https://www.douyin.com/video/6961737553342991651").await.unwrap_err();
    println!("douyin: [{}] {e} / {:?}", e.kind(), e.detail());
    let leftovers = std::fs::read_dir(std::env::temp_dir().join("rabbit-sessions")).map(|rd| rd.count()).unwrap_or(0);
    assert_eq!(leftovers, 0, "temporary session files must be deleted");
}
/// Diagnostics for the signed-in route: which CDN host each Bilibili format comes from (hosts only).
#[tokio::test]
#[ignore = "needs your signed-in Firefox profile (RABBIT_SESSION_SOURCE)"]
async fn session_bilibili_hosts() {
    use rabbit_lib::extract::web::{SessionCookies, Web};
    use rabbit_lib::sys::cookies::{read, CookieSource, Site};
    let Ok(raw) = std::env::var("RABBIT_SESSION_SOURCE") else { return };
    let cookies = read(&CookieSource::FirefoxProfile { path: raw.into() }, Site::Bilibili).unwrap();
    let session = SessionCookies::new(Site::Bilibili, &cookies, rabbit_lib::extract::bilibili::SESSION_HOSTS).unwrap();
    let web = Web::new().unwrap();
    let url = url::Url::parse(BILI).unwrap();
    let v = rabbit_lib::extract::bilibili::extract(&web, &url, Some(&session)).await.unwrap().unwrap();
    for f in v["formats"].as_array().unwrap() {
        let host = |u: &serde_json::Value| u.as_str().and_then(|u| url::Url::parse(u).ok()).and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
        let backups: Vec<String> = f["backup_urls"].as_array().map(|a| a.iter().map(host).collect()).unwrap_or_default();
        println!("{:<10} {:>5}p  {}  backups: {:?}", f["format_id"].as_str().unwrap_or(""), f["height"], host(&f["url"]), backups);
    }
    // Request variants against the 1080p stream: which header makes the difference?
    for (id, which) in [("80-7", 1usize), ("32-7", 0), ("64-7", 0)] {
        let Some(f) = v["formats"].as_array().unwrap().iter().find(|f| f["format_id"] == id) else { continue };
        let u = if which == 0 { f["url"].as_str().unwrap().to_string() } else { f["backup_urls"][0].as_str().unwrap().to_string() };
        println!("-- {id} via {}", url::Url::parse(&u).unwrap().host_str().unwrap());
        let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).timeout(std::time::Duration::from_secs(30)).build().unwrap();
        for (label, range, identity) in [("0-", "bytes=0-", true), ("1048576-", "bytes=1048576-", true)] {
            let mut rb = client.get(&u).header("Referer", "https://www.bilibili.com/").header("User-Agent", rabbit_lib::extract::web::DESKTOP_UA).header("Range", range);
            if identity {
                rb = rb.header("Accept-Encoding", "identity");
            }
            let t = Instant::now();
            match rb.send().await {
                Ok(r) => {
                    let status = r.status();
                    let mut got = 0usize;
                    let mut s = r.bytes_stream();
                    use futures_util::StreamExt;
                    let mut err = None;
                    while let Some(chunk) = s.next().await {
                        match chunk { Ok(b) => { got += b.len(); if got > 2_000_000 { break } } Err(e) => { err = Some(e.without_url().to_string()); break } }
                    }
                    println!("variant {label:<16} HTTP {status} body {got} B in {:.1}s {:?}", t.elapsed().as_secs_f64(), err);
                }
                Err(e) => println!("variant {label:<16} ERROR {} after {:.1}s", e.without_url(), t.elapsed().as_secs_f64()),
            }
        }
    }
    // The engine on the 1080p stream with different widths.
    if let Some(f) = v["formats"].as_array().unwrap().iter().find(|f| f["format_id"] == "80-7") {
        let u = f["url"].as_str().unwrap().to_string();
        let headers: std::collections::BTreeMap<String, String> = f["http_headers"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        let client = reqwest::Client::new();
        for segs in [1, 4] {
            let dir = tempfile::tempdir().unwrap();
            let req = rabbit_lib::engine::http::Request { client: &client, url: &u, headers: rabbit_lib::engine::http::build_headers(&headers), max_segments: segs, max_attempts: rabbit_lib::engine::http::MAX_ATTEMPTS };
            let t = Instant::now();
            let r = rabbit_lib::engine::http::fetch(&req, &dir.path().join("v.m4s"), Arc::new(rabbit_lib::engine::http::Counters::default()), &CancellationToken::new(), None).await;
            println!("engine 80-7 segments={segs}: {:?} in {:.1}s", r.map_err(|e| e.to_string()), t.elapsed().as_secs_f64());
        }
    }
    // Probe the main and backup URL of the 1080p and 480p H.264 streams: status / error chain only.
    let client = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(15)).build().unwrap();
    for id in ["80-7", "32-7"] {
        let Some(f) = v["formats"].as_array().unwrap().iter().find(|f| f["format_id"] == id) else { continue };
        let urls = std::iter::once(f["url"].as_str().unwrap().to_string()).chain(f["backup_urls"].as_array().into_iter().flatten().filter_map(|u| u.as_str().map(str::to_string)));
        for u in urls {
            let host = url::Url::parse(&u).unwrap().host_str().unwrap().to_string();
            let r = client.get(&u).header("Referer", "https://www.bilibili.com/").header("User-Agent", rabbit_lib::extract::web::DESKTOP_UA).header("Range", "bytes=0-0").send().await;
            match r {
                Ok(r) => println!("probe {id} {host}: HTTP {} {:?}", r.status(), r.headers().get("content-range")),
                Err(e) => {
                    let e = e.without_url();
                    let mut chain = vec![e.to_string()];
                    let mut src = std::error::Error::source(&e);
                    while let Some(s) = src {
                        chain.push(s.to_string());
                        src = s.source();
                    }
                    println!("probe {id} {host}: ERROR {}", chain.join(" <- "));
                }
            }
        }
    }
}