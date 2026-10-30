//! Transfer-engine throughput against real CDNs, by number of parallel segments (docs/TEST_RESULTS.md).
//! Each source is extracted fresh, then the same file is fetched once per setting, sequentially.
//!   cargo test --release --test bench_transfer -- --ignored --nocapture --test-threads=1

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use rabbit_lib::engine::http::{build_headers, fetch, Counters, Request};
use rabbit_lib::extract::web::Web;
use rabbit_lib::extract::{ytdlp, Native};
use rabbit_lib::engine::platform::Platform;
use rabbit_lib::sys::tools::Tools;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

fn tools() -> Tools {
    let mut t = Tools::discover();
    t.ytdlp = Some(Path::new(env!("CARGO_MANIFEST_DIR")).join(if cfg!(windows) { "binaries/yt-dlp-x86_64-pc-windows-msvc.exe" } else { "binaries/linux/yt-dlp" }));
    t
}

/// The direct-HTTPS format with this id (or the largest direct one), with its headers.
fn pick(v: &Value, id: Option<&str>) -> (String, BTreeMap<String, String>, String) {
    let formats = v["formats"].as_array().unwrap();
    let direct = formats.iter().filter(|f| f["protocol"] == "https" && f["url"].is_string());
    let f = match id {
        Some(id) => direct.clone().find(|f| f["format_id"] == id).unwrap_or_else(|| {
            let have: Vec<String> = formats.iter().map(|f| format!("{}:{}:{}", f["format_id"], f["protocol"], f["url"].is_string())).collect();
            panic!("format {id} not among {have:?}")
        }),
        None => direct.max_by_key(|f| f["filesize"].as_u64().or(f["filesize_approx"].as_u64()).unwrap_or(0)).unwrap(),
    };
    let headers = f["http_headers"].as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
    (f["url"].as_str().unwrap().to_string(), headers, f["format_id"].as_str().unwrap_or("?").to_string())
}

async fn bench(label: &str, url: &str, headers: &BTreeMap<String, String>) {
    let client = reqwest::Client::builder().tcp_nodelay(true).build().unwrap();
    for segments in [1, 2, 4, 8, 16] {
        let dir = tempfile::tempdir().unwrap();
        let req = Request { client: &client, url, headers: build_headers(headers), max_segments: segments, max_attempts: rabbit_lib::engine::http::MAX_ATTEMPTS };
        let t = Instant::now();
        let r = fetch(&req, &dir.path().join("f.bin"), Arc::new(Counters::default()), &CancellationToken::new(), None).await;
        let secs = t.elapsed().as_secs_f64();
        match r {
            Ok(bytes) => println!("{label:<34} segments={segments:<2} {:>7.1} MB in {secs:>6.2} s = {:>6.2} MB/s", bytes as f64 / 1e6, bytes as f64 / 1e6 / secs),
            Err(e) => println!("{label:<34} segments={segments:<2} FAILED after {secs:.1} s: {e}"),
        }
    }
}

#[tokio::test]
#[ignore = "network"]
async fn throughput_by_segment_count() {
    let tools = tools();
    let cancel = CancellationToken::new();
    // YouTube: Big Buck Bunny 1080p60 H.264 video-only (format 299, ≈ 200 MB), Google's CDN.
    let yt = ytdlp::dump_json(&tools, "https://www.youtube.com/watch?v=aqz-KE-bpKQ", &cancel).await.unwrap();
    let (url, headers, id) = pick(&yt, Some("299"));
    bench(&format!("YouTube f{id}"), &url, &headers).await;

    let web = Web::new().unwrap();
    // Bilibili: the largest DASH stream of a 9-minute video (Akamai).
    let u = url::Url::parse("https://www.bilibili.com/video/BV13x41117TL").unwrap();
    let bili = Native::for_url(Platform::Bilibili, &u).unwrap().extract(&web, &u, None).await.unwrap().unwrap();
    let (url, headers, id) = pick(&bili, None);
    bench(&format!("Bilibili {id}"), &url, &headers).await;

    // Kuaishou: the largest rendition of a 2-minute video.
    let u = url::Url::parse("https://www.kuaishou.com/short-video/3x88nbkyjtga6zk").unwrap();
    let ks = Native::for_url(Platform::Kuaishou, &u).unwrap().extract(&web, &u, None).await.unwrap().unwrap();
    let (url, headers, id) = pick(&ks, None);
    bench(&format!("Kuaishou {id}"), &url, &headers).await;
}
