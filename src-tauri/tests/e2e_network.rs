//! Live end-to-end tests against public media. Ignored by default (need network):
//!   cargo test --test e2e_network -- --ignored --nocapture --test-threads=1
//! Output goes to a temp folder; the user's settings and library are untouched.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rabbit_lib::engine::manager::{Manager, TaskState};
use rabbit_lib::engine::post;
use rabbit_lib::extract::model::OptionGroup;
use rabbit_lib::store::{Settings, Store};
use rabbit_lib::sys::tools::Tools;
use tokio_util::sync::CancellationToken;

const ZOO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw"; // "Me at the zoo", 19 s, public since 2005

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

/// Big Buck Bunny (Blender Foundation, CC BY 3.0), 10 min — large enough to pause mid-transfer.
const BBB: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";

async fn wait_for(m: &Manager, id: &str, limit: Duration, cond: impl Fn(&rabbit_lib::engine::manager::TaskInfo) -> bool) -> rabbit_lib::engine::manager::TaskInfo {
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

#[tokio::test]
#[ignore = "network"]
async fn interrupted_download_resumes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let a = manager(dir.path());
    let info = a.analyze(BBB).await.expect("analyze");
    let opt = info
        .options
        .iter()
        .filter(|o| o.note.starts_with("merges") && !o.note.contains("segmented") && o.size.is_some_and(|s| s > 60_000_000))
        .min_by_key(|o| o.size.unwrap())
        .expect("a native merge option over 60 MB");
    let expected = opt.size.unwrap();
    println!("option {} {} ≈{} bytes", opt.id, opt.label, expected);
    let id = a.enqueue(&info.analysis_id, &opt.id).unwrap().id;

    // Let some data land, then "restart": a second manager reads the queue from the same database.
    wait_for(&a, &id, Duration::from_secs(120), |t| t.done > 3_000_000).await;
    tokio::time::sleep(Duration::from_millis(1200)).await; // at least one resume-state write
    let b = manager(dir.path());
    a.pause(&id).unwrap(); // stop the "old process" before the new one resumes
    wait_for(&a, &id, Duration::from_secs(30), |t| t.state == TaskState::Paused).await;

    let restored = b.list().into_iter().find(|t| t.id == id).expect("task restored from the database");
    println!("restored: {:?} interrupted={} saved={} bytes", restored.state, restored.interrupted, restored.done);
    assert_eq!(restored.state, TaskState::Paused);
    assert!(restored.interrupted, "a task that was running at exit is flagged as interrupted");
    assert!(restored.done > 1_000_000, "saved bytes are reported before resuming");
    assert!((restored.done as f64) < expected as f64 * 0.9, "restart must happen mid-file for this test to mean anything");

    // Resume in the "new process": re-analyzes for fresh links and continues from the saved bytes.
    let t0 = Instant::now();
    b.resume(&id).unwrap();
    let done = wait_for(&b, &id, Duration::from_secs(600), |t| !matches!(t.state, TaskState::Queued | TaskState::Downloading | TaskState::Processing | TaskState::Verifying)).await;
    assert_eq!(done.state, TaskState::Completed, "{:?}", done.error);
    println!("resumed run finished in {:.1} s", t0.elapsed().as_secs_f64());

    let path = Path::new(done.file_path.as_deref().unwrap()).to_path_buf();
    let probe = post::probe(&tools(), &path, &CancellationToken::new()).await.unwrap();
    println!("{} → {:.1} s, {} video / {} audio", path.file_name().unwrap().to_string_lossy(), probe.duration.unwrap(), probe.video, probe.audio);
    assert!((probe.duration.unwrap() - info.duration.unwrap()).abs() < 3.0);
    assert!(probe.video >= 1 && probe.audio >= 1);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("out")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".rabbit-")).collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
}

#[tokio::test]
#[ignore = "network"]
async fn pause_resume_and_cancel_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let info = m.analyze(BBB).await.expect("analyze");
    let opt = info
        .options
        .iter()
        .filter(|o| o.note.starts_with("merges") && !o.note.contains("segmented") && o.size.is_some_and(|s| s > 15_000_000))
        .min_by_key(|o| o.size.unwrap())
        .unwrap();
    let out = dir.path().join("out");
    let temps = || std::fs::read_dir(&out).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".rabbit-")).count()).unwrap_or(0);

    // Pause keeps partial data.
    let id = m.enqueue(&info.analysis_id, &opt.id).unwrap().id;
    wait_for(&m, &id, Duration::from_secs(120), |t| t.done > 2_000_000).await;
    m.pause(&id).unwrap();
    let paused = wait_for(&m, &id, Duration::from_secs(30), |t| t.state == TaskState::Paused).await;
    assert!(temps() > 0, "pause must keep partial files");
    println!("paused at {} bytes, {} temp files", paused.done, temps());

    // Cancelling a paused task discards its partial data.
    m.cancel(&id).unwrap();
    let cancelled = wait_for(&m, &id, Duration::from_secs(10), |t| t.state == TaskState::Cancelled).await;
    assert_eq!(cancelled.state, TaskState::Cancelled);
    assert_eq!(temps(), 0, "cancel must delete partial files");

    // Removing a task forgets it in the database too.
    m.remove(&id).unwrap();
    let again = manager(dir.path());
    assert!(again.list().iter().all(|t| t.id != id));
}

async fn wait_terminal(m: &Manager, ids: &[String], limit: Duration) {
    let start = Instant::now();
    loop {
        let all = m.list();
        let done = ids.iter().all(|id| all.iter().any(|t| &t.id == id && !matches!(t.state, TaskState::Queued | TaskState::Downloading | TaskState::Processing | TaskState::Verifying)));
        if done {
            return;
        }
        assert!(start.elapsed() < limit, "timed out: {:?}", all.iter().map(|t| (&t.option_label, t.state)).collect::<Vec<_>>());
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

#[tokio::test]
#[ignore = "network"]
async fn youtube_video_and_mp3_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());

    let t0 = Instant::now();
    let info = m.analyze(&format!("look at this {ZOO} !")).await.expect("analyze");
    println!("analyze: {} ms, {} options", t0.elapsed().as_millis(), info.options.len());
    assert_eq!(info.title, "Me at the zoo");
    assert!(info.duration.is_some_and(|d| (d - 19.0).abs() < 1.5));
    assert!(info.thumbnail.as_deref().is_some_and(|t| t.starts_with("data:image/")));

    let video = info.options.iter().find(|o| o.group == OptionGroup::Video && o.recommended).expect("recommended video");
    let mp3 = info.options.iter().find(|o| o.container == "mp3").expect("mp3 option");
    let cover = info.options.iter().find(|o| o.id == "cover").expect("cover option");
    let merged = info.options.iter().find(|o| o.note.starts_with("merges") && !o.note.contains("segmented")).expect("a native video+audio merge option");
    for o in &info.options {
        println!("  option {:<28} {:<6} {:<5} {:<16} {}", o.id, o.label, o.container, o.codec, o.note);
    }
    let picks = [video, mp3, cover, merged];
    let ids: Vec<String> = picks.iter().map(|o| m.enqueue(&info.analysis_id, &o.id).unwrap().id).collect();
    wait_terminal(&m, &ids, Duration::from_secs(180)).await;

    let tools = tools();
    let cancel = CancellationToken::new();
    for t in m.list() {
        assert_eq!(t.state, TaskState::Completed, "{} failed: {:?}", t.option_label, t.error);
        let p = Path::new(t.file_path.as_deref().unwrap()).to_path_buf();
        assert!(p.starts_with(dir.path().join("out")), "written outside the download folder: {}", p.display());
        let size = std::fs::metadata(&p).unwrap().len();
        println!("{} → {} ({} bytes)", t.option_label, p.file_name().unwrap().to_string_lossy(), size);
        if t.container != "jpg" && t.container != "webp" {
            let probe = post::probe(&tools, &p, &cancel).await.unwrap();
            assert!((probe.duration.unwrap() - 19.0).abs() < 2.0);
            assert!(probe.audio >= 1);
        }
    }
    // Same title for every task: names must not collide.
    let names: std::collections::HashSet<_> = m.list().iter().map(|t| t.file_path.clone()).collect();
    assert_eq!(names.len(), picks.len());
    // No temp files may remain.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("out")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(".rabbit-")).collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
}

#[tokio::test]
#[ignore = "network"]
async fn unsupported_and_invalid_inputs_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let m = manager(dir.path());
    let e = m.analyze("no links here").await.unwrap_err();
    assert_eq!(e.kind(), "input");
    let e = m.analyze("http://192.168.0.1/admin").await.unwrap_err();
    assert_eq!(e.kind(), "input");
    let e = m.analyze("https://www.youtube.com/watch?v=00000000000").await.unwrap_err();
    println!("unavailable video → {} / {}", e.kind(), e);
    assert!(matches!(e.kind(), "extractor" | "unsupported"));
}
