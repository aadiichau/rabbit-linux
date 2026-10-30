//! Platform compatibility probe (Phase 6). Runs every URL in tests/platform-urls.json through the
//! real pipeline — analyze, pick an option, download, verify — into a temp folder, and writes
//! research/platform-report.json.
//!   cargo test --test platform_matrix -- --ignored --nocapture
//!   RABBIT_PLATFORMS=suno,kuaishou cargo test --test platform_matrix -- --ignored --nocapture

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rabbit_lib::engine::manager::{Manager, TaskState};
use rabbit_lib::engine::post;
use rabbit_lib::extract::model::{DownloadOption, MediaKind, OptionGroup};
use rabbit_lib::store::{Settings, Store};
use rabbit_lib::sys::tools::Tools;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const MAX_BYTES: u64 = 40_000_000;

fn tools() -> Tools {
    let mut t = Tools::discover();
    t.ytdlp = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) { "binaries/yt-dlp-x86_64-pc-windows-msvc.exe" } else { "binaries/linux/yt-dlp" }));
    t
}

fn short_side(o: &DownloadOption) -> u32 {
    match (o.width, o.height) {
        (Some(w), Some(h)) => w.min(h),
        (_, h) => h.unwrap_or(0),
    }
}

/// `RABBIT_PICK=best`: the highest-resolution video option (quality audit), else the option a user
/// would most likely take, kept small enough for a test run.
fn choose(options: &[DownloadOption], kind: MediaKind) -> Option<&DownloadOption> {
    if kind == MediaKind::Video && std::env::var("RABBIT_PICK").as_deref() == Ok("best") {
        return options.iter().filter(|o| o.group == OptionGroup::Video && o.id != "gallery-all").max_by_key(|o| (short_side(o), o.size.unwrap_or(0)));
    }
    let group = match kind {
        MediaKind::Video => OptionGroup::Video,
        MediaKind::Audio => OptionGroup::Audio,
        MediaKind::Image | MediaKind::Gallery => OptionGroup::Image,
    };
    let in_group: Vec<&DownloadOption> = options.iter().filter(|o| o.group == group || o.id == "gallery-all").collect();
    let pool = if in_group.is_empty() { options.iter().collect() } else { in_group };
    let rec = pool.iter().copied().find(|o| o.recommended).or_else(|| pool.first().copied())?;
    if rec.size.is_some_and(|s| s > MAX_BYTES) {
        // Smallest option that still has video+audio when this is a video.
        return pool.iter().copied().filter(|o| !o.conversion).min_by_key(|o| o.size.unwrap_or(u64::MAX));
    }
    Some(rec)
}

#[tokio::test]
#[ignore = "network"]
async fn platform_matrix() {
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/platform-urls.json")).unwrap()).unwrap();
    let only: Option<Vec<String>> = std::env::var("RABBIT_PLATFORMS").ok().map(|s| s.split(',').map(str::to_string).collect());
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&dir.path().join("t.db")).unwrap());
    let m = Manager::new(store, Settings { download_dir: dir.path().join("out"), max_concurrent: 1, max_segments: 8, notifications: false, ytdlp_channel: Default::default(), sessions: Default::default() }, tools()).unwrap();
    let tools = tools();

    let mut report = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let platform = case["platform"].as_str().unwrap();
        if only.as_ref().is_some_and(|o| !o.iter().any(|p| p == platform)) {
            continue;
        }
        let url = case["url"].as_str().unwrap();
        let t0 = Instant::now();
        let mut row = json!({ "platform": platform, "url": url, "source": case["source"], "expect": case["expect"] });
        match m.analyze(url).await {
            Err(e) => {
                row["stage"] = json!("analyze");
                row["ok"] = json!(false);
                row["errorKind"] = json!(e.kind());
                row["error"] = json!(e.to_string());
            }
            Ok(info) => {
                row["analyzeMs"] = json!(t0.elapsed().as_millis() as u64);
                row["kind"] = json!(info.kind);
                row["title"] = json!(info.title);
                row["extractor"] = json!(info.extractor);
                row["itemCount"] = json!(info.item_count);
                row["hasThumbnail"] = json!(info.thumbnail.is_some());
                row["options"] = json!(info.options.iter().map(|o| format!("{} {} {}", o.label, o.container, o.codec)).collect::<Vec<_>>());
                row["bestOffered"] = json!(info.options.iter().filter(|o| o.group == OptionGroup::Video).map(short_side).max());
                match choose(&info.options, info.kind) {
                    None => {
                        row["stage"] = json!("choose");
                        row["ok"] = json!(false);
                    }
                    Some(opt) => {
                        row["chosen"] = json!(format!("{} ({})", opt.label, opt.id));
                        let id = m.enqueue(&info.analysis_id, &opt.id).unwrap().id;
                        let start = Instant::now();
                        let t = loop {
                            let t = m.list().into_iter().find(|t| t.id == id).unwrap();
                            if !matches!(t.state, TaskState::Queued | TaskState::Downloading | TaskState::Processing | TaskState::Verifying) || start.elapsed() > Duration::from_secs(900) {
                                break t;
                            }
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        };
                        row["downloadMs"] = json!(start.elapsed().as_millis() as u64);
                        row["state"] = json!(t.state);
                        row["ok"] = json!(t.state == TaskState::Completed);
                        row["stage"] = json!("download");
                        if let Some(e) = &t.error {
                            row["errorKind"] = json!(e.kind);
                            row["error"] = json!(e.message);
                        }
                        if let Some(p) = &t.file_path {
                            let p = Path::new(p);
                            let bytes = if p.is_dir() {
                                std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.metadata().map(|m| m.len()).unwrap_or(0)).sum()).unwrap_or(0)
                            } else {
                                std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
                            };
                            row["bytes"] = json!(bytes);
                            row["file"] = json!(p.file_name().map(|n| n.to_string_lossy().to_string()));
                            if p.is_dir() {
                                // Every item, with the size and streams FFprobe reads from it.
                                let mut files = Vec::new();
                                for e in std::fs::read_dir(p).unwrap().flatten() {
                                    let pr = post::probe(&tools, &e.path(), &CancellationToken::new()).await.ok();
                                    files.push(json!({
                                        "name": e.file_name().to_string_lossy(),
                                        "bytes": e.metadata().map(|m| m.len()).unwrap_or(0),
                                        "width": pr.as_ref().and_then(|p| p.width),
                                        "height": pr.as_ref().and_then(|p| p.height),
                                        "vcodec": pr.as_ref().and_then(|p| p.vcodec.clone()),
                                        "acodec": pr.as_ref().and_then(|p| p.acodec.clone()),
                                    }));
                                }
                                row["files"] = json!(files);
                            } else if let Ok(pr) = post::probe(&tools, p, &CancellationToken::new()).await {
                                row["probe"] = json!({ "duration": pr.duration, "video": pr.video, "audio": pr.audio, "width": pr.width, "height": pr.height, "vcodec": pr.vcodec, "acodec": pr.acodec });
                            }
                        }
                    }
                }
            }
        }
        println!(
            "{:<10} {:<5} {:<9} {:<10} {}",
            platform,
            if row["ok"] == json!(true) { "OK" } else { "FAIL" },
            row["kind"].as_str().unwrap_or("-"),
            row["errorKind"].as_str().unwrap_or(""),
            row["error"].as_str().map(|e| e.chars().take(110).collect::<String>()).unwrap_or_else(|| format!(
                "{} · {} B · offered ≤{}p · file {}x{} {}+{}",
                row["chosen"].as_str().unwrap_or(""),
                row["bytes"],
                row["bestOffered"],
                row["probe"]["width"],
                row["probe"]["height"],
                row["probe"]["vcodec"].as_str().unwrap_or("-"),
                row["probe"]["acodec"].as_str().unwrap_or("-")
            )),
        );
        report.push(row);
    }
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../research/platform-report.json");
    let existing: Vec<Value> = std::fs::read_to_string(&out).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // Replace rows for re-run URLs, keep the others.
    let mut merged: Vec<Value> = existing.into_iter().filter(|r| !report.iter().any(|n| n["url"] == r["url"])).collect();
    merged.extend(report);
    std::fs::write(&out, serde_json::to_string_pretty(&merged).unwrap()).unwrap();
    println!("report: {}", out.display());
}
