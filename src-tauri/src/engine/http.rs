//! Native HTTP transfer: single stream or parallel byte-range segments written in place.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT_ENCODING, CONTENT_RANGE, RANGE, RETRY_AFTER};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};

const STALL_TIMEOUT: Duration = Duration::from_secs(25);
/// Attempts per request before a transfer gives up. With a mirror to fall back to, callers pass
/// fewer, so a host that keeps failing is abandoned quickly (`Request::max_attempts`).
pub const MAX_ATTEMPTS: u32 = 6;
/// Bytes one attempt must deliver before a later failure no longer counts against the budget.
const PROGRESS_RESETS_RETRIES: u64 = 2 * 1024 * 1024;
const WRITE_BUFFER: usize = 512 * 1024;
/// Files smaller than this are fetched as one stream; segmenting them costs more than it gains.
pub const MIN_SPLIT_SIZE: u64 = 4 * 1024 * 1024;
/// Smallest segment worth its own connection. Measured (docs/TEST_RESULTS.md §4): a 22 MB file was
/// fastest with 4 connections and slower with 16, while a 258 MB YouTube stream scaled up to 16.
const MIN_SEGMENT: u64 = 4 * 1024 * 1024;

#[derive(Default)]
pub struct Counters {
    pub done: AtomicU64,
    /// 0 while unknown.
    pub total: AtomicU64,
    pub segments: Mutex<Vec<Arc<Segment>>>,
}

pub struct Segment {
    pub start: u64,
    /// `u64::MAX` when the total size is unknown.
    pub len: u64,
    /// Bytes received (may still sit in the write buffer).
    pub done: AtomicU64,
    /// Bytes handed to the OS (not in Rabbit's write buffer) — the only figure written to resume state.
    pub persisted: AtomicU64,
}

impl Counters {
    pub fn segment_fractions(&self) -> Vec<f32> {
        let segs = self.segments.lock().unwrap();
        if segs.len() < 2 {
            return Vec::new();
        }
        segs.iter().map(|s| (s.done.load(Ordering::Relaxed) as f64 / s.len.max(1) as f64).min(1.0) as f32).collect()
    }
}

pub fn build_headers(h: &BTreeMap<String, String>) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in h {
        let lk = k.to_ascii_lowercase();
        if matches!(lk.as_str(), "host" | "range" | "content-length" | "accept-encoding" | "connection") {
            continue;
        }
        if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
            map.insert(name, val);
        }
    }
    // Byte offsets must refer to the stored representation.
    map.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
    map
}

fn status_error(resp: &Response) -> AppError {
    let code = resp.status().as_u16();
    match resp.status() {
        StatusCode::FORBIDDEN | StatusCode::GONE | StatusCode::UNAUTHORIZED => AppError::Expired(code),
        _ => AppError::Http(code),
    }
}

fn parse_total(resp: &Response) -> Option<u64> {
    // "bytes 0-1023/146515"
    resp.headers().get(CONTENT_RANGE)?.to_str().ok()?.rsplit('/').next()?.parse().ok()
}

fn retry_after(resp: &Response) -> Option<Duration> {
    let secs: u64 = resp.headers().get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs.min(120)))
}

fn backoff(attempt: u32) -> Duration {
    let base = 500u64.saturating_mul(1 << attempt.min(6)).min(30_000);
    let jitter = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(0) % (base / 2 + 1);
    Duration::from_millis(base / 2 + jitter)
}

async fn sleep_or_cancel(d: Duration, cancel: &CancellationToken) -> AppResult<()> {
    tokio::select! {
        _ = tokio::time::sleep(d) => Ok(()),
        _ = cancel.cancelled() => Err(AppError::Cancelled),
    }
}

// Unix files already support sparse gaps after set_len/seek; no NTFS-style flag is needed.
#[cfg(not(windows))]
fn set_sparse(_file: &std::fs::File, _sparse: bool) {}

#[cfg(windows)]
fn set_sparse(file: &std::fs::File, sparse: bool) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{FILE_SET_SPARSE_BUFFER, FSCTL_SET_SPARSE};
    let buf = FILE_SET_SPARSE_BUFFER { SetSparse: sparse };
    let mut returned = 0u32;
    // Failure only costs performance (NTFS zero-fills gaps); it never affects correctness.
    unsafe {
        DeviceIoControl(
            file.as_raw_handle() as _,
            FSCTL_SET_SPARSE,
            &buf as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<FILE_SET_SPARSE_BUFFER>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        );
    }
}

pub struct Request<'a> {
    pub client: &'a reqwest::Client,
    pub url: &'a str,
    pub headers: HeaderMap,
    pub max_segments: usize,
    pub max_attempts: u32,
}

/// On-disk resume record kept next to a partial file. Only flushed bytes are recorded,
/// so a crash can never claim data that isn't in the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResumeState {
    pub total: u64,
    pub etag: Option<String>,
    /// (start, len, persisted)
    pub segments: Vec<(u64, u64, u64)>,
    #[serde(default)]
    pub complete: bool,
}

impl ResumeState {
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    fn save(&self, path: &Path) {
        let tmp = path.with_extension("tmp");
        if let Ok(bytes) = serde_json::to_vec(self) {
            if std::fs::write(&tmp, bytes).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }

    fn snapshot(total: u64, etag: &Option<String>, segs: &[Arc<Segment>], complete: bool) -> Self {
        ResumeState {
            total,
            etag: etag.clone(),
            segments: segs.iter().map(|s| (s.start, s.len, s.persisted.load(Ordering::Relaxed))).collect(),
            complete,
        }
    }
}

fn etag_of(resp: &Response) -> Option<String> {
    resp.headers().get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(|s| s.trim_start_matches("W/").to_string())
}

/// Downloads `url` into `dest`. With `state_path`, progress is recorded so a later call can
/// resume a ranged transfer where it stopped (pause, crash, or app restart).
pub async fn fetch(req: &Request<'_>, dest: &Path, counters: Arc<Counters>, cancel: &CancellationToken, state_path: Option<&Path>) -> AppResult<u64> {
    let previous = state_path.and_then(ResumeState::load).filter(|s| std::fs::metadata(dest).is_ok_and(|m| m.len() == s.total));
    if let Some(s) = previous.as_ref().filter(|s| s.complete) {
        // This stream finished in an earlier run (e.g. paused during the merge step).
        counters.total.store(s.total, Ordering::Relaxed);
        counters.done.store(s.total, Ordering::Relaxed);
        return Ok(s.total);
    }

    // Probe with an open-ended range: a 206 tells us ranges work and gives the size,
    // and its body is reused as the first segment so no round trip is wasted.
    let mut attempt = 0;
    let first = loop {
        let sent = tokio::select! {
            r = req.client.get(req.url).headers(req.headers.clone()).header(RANGE, "bytes=0-").send() => r,
            _ = cancel.cancelled() => return Err(AppError::Cancelled),
        };
        match sent {
            Ok(r) if r.status().is_success() => break r,
            Ok(r) if r.status() == StatusCode::RANGE_NOT_SATISFIABLE => {
                // Zero-length resource.
                tokio::fs::write(dest, b"").await?;
                return Ok(0);
            }
            Ok(r) if (r.status() == StatusCode::TOO_MANY_REQUESTS || r.status().is_server_error()) && attempt < req.max_attempts => {
                let wait = retry_after(&r).unwrap_or_else(|| backoff(attempt));
                attempt += 1;
                sleep_or_cancel(wait, cancel).await?;
            }
            Ok(r) => return Err(status_error(&r)),
            Err(e) if attempt < req.max_attempts && !e.is_builder() => {
                attempt += 1;
                sleep_or_cancel(backoff(attempt), cancel).await?;
            }
            Err(e) => return Err(e.into()),
        }
    };

    let ranged = first.status() == StatusCode::PARTIAL_CONTENT;
    let total = if ranged { parse_total(&first) } else { first.content_length() };
    let etag = etag_of(&first);
    counters.total.store(total.unwrap_or(0), Ordering::Relaxed);

    // Resume only when the server still serves the same bytes.
    let resume = previous.filter(|p| {
        ranged
            && total == Some(p.total)
            && match (&p.etag, &etag) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    });

    let segments: Vec<Arc<Segment>> = if let Some(p) = &resume {
        p.segments
            .iter()
            .map(|&(start, len, persisted)| {
                let done = persisted.min(len);
                Arc::new(Segment { start, len, done: AtomicU64::new(done), persisted: AtomicU64::new(done) })
            })
            .collect()
    } else {
        let file = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(dest)?;
        let n_segments = match total {
            Some(t) if ranged && t >= MIN_SPLIT_SIZE && req.max_segments > 1 => ((t / MIN_SEGMENT) as usize).clamp(1, req.max_segments),
            _ => 1,
        };
        if n_segments > 1 {
            set_sparse(&file, true);
        }
        if let Some(t) = total {
            file.set_len(t)?;
        }
        drop(file);
        match total {
            Some(t) => {
                let size = t.div_ceil(n_segments as u64);
                (0..n_segments as u64)
                    .map(|i| {
                        let start = i * size;
                        Arc::new(Segment { start, len: size.min(t - start), done: AtomicU64::new(0), persisted: AtomicU64::new(0) })
                    })
                    .filter(|s| s.len > 0)
                    .collect()
            }
            None => vec![Arc::new(Segment { start: 0, len: u64::MAX, done: AtomicU64::new(0), persisted: AtomicU64::new(0) })],
        }
    };
    let resumed_bytes: u64 = segments.iter().map(|s| s.done.load(Ordering::Relaxed)).sum();
    counters.done.store(resumed_bytes, Ordering::Relaxed);
    *counters.segments.lock().unwrap() = segments.clone();
    if resumed_bytes > 0 {
        tracing::info!(resumed_bytes, "resuming partial transfer");
    }

    // Periodic state writer (ranged transfers with a known size only).
    let persist = match (state_path, total) {
        (Some(p), Some(t)) if ranged => Some((p.to_path_buf(), t)),
        _ => None,
    };
    let saver_stop = CancellationToken::new();
    let saver = persist.clone().map(|(path, t)| {
        let segs = segments.clone();
        let etag = etag.clone();
        let stop = saver_stop.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => ResumeState::snapshot(t, &etag, &segs, false).save(&path),
                    _ = stop.cancelled() => break,
                }
            }
        })
    });

    let child = cancel.child_token();
    let gate = Arc::new(Gate::new(segments.len()));
    let mut tasks = tokio::task::JoinSet::new();
    let mut first = Some(first);
    for seg in segments.iter().cloned() {
        // The probe body starts at byte 0, so it can only serve a segment that hasn't started.
        let initial = if seg.start == 0 && seg.done.load(Ordering::Relaxed) == 0 { first.take() } else { None };
        let client = req.client.clone();
        let url = req.url.to_string();
        let headers = req.headers.clone();
        let dest = dest.to_path_buf();
        let counters = counters.clone();
        let cancel = child.clone();
        let gate = gate.clone();
        let max_attempts = req.max_attempts;
        tasks.spawn(async move { run_segment(&client, &url, headers, &dest, seg, initial, ranged, &counters, &gate, max_attempts, &cancel).await });
    }

    drop(first);
    let mut failure: Option<AppError> = None;
    while let Some(res) = tasks.join_next().await {
        let res = res.unwrap_or_else(|e| Err(AppError::Io(format!("transfer task failed: {e}"))));
        if let Err(e) = res {
            if failure.is_none() {
                child.cancel(); // stop the other segments
                failure = Some(e);
            }
        }
    }
    saver_stop.cancel();
    if let Some(s) = saver {
        let _ = s.await;
    }
    let written = counters.done.load(Ordering::Relaxed);
    let ok = failure.is_none() && total.is_none_or(|t| written == t);
    if let Some((path, t)) = &persist {
        // Final, exact snapshot (all segments have flushed by now).
        ResumeState::snapshot(*t, &etag, &segments, ok).save(path);
    }
    if let Some(e) = failure {
        return Err(if cancel.is_cancelled() { AppError::Cancelled } else { e });
    }
    if let Some(t) = total {
        if written != t {
            return Err(AppError::Verification(format!("expected {t} bytes, received {written}")));
        }
    } else {
        counters.total.store(written, Ordering::Relaxed);
    }
    if segments.len() > 1 {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(dest) {
            set_sparse(&f, false);
        }
    }
    Ok(written)
}

/// `fetch` over a list of hosts serving the same file (the source's backup URLs), in order. A host
/// that keeps failing is given fewer attempts while another can take over — Bilibili's Akamai mirror
/// cut HD transfers after ~270 KB while its bilivideo.com backup served them in full. Partial data
/// carries over when the next host serves identical bytes (the resume state checks size and ETag).
#[allow(clippy::too_many_arguments)]
pub async fn fetch_mirrored(
    client: &reqwest::Client,
    urls: &[&str],
    headers: &HeaderMap,
    max_segments: usize,
    dest: &Path,
    counters: Arc<Counters>,
    cancel: &CancellationToken,
    state_path: Option<&Path>,
) -> AppResult<u64> {
    let mut last = None;
    for (i, url) in urls.iter().enumerate() {
        let more = i + 1 < urls.len();
        let req = Request { client, url, headers: headers.clone(), max_segments, max_attempts: if more { 2 } else { MAX_ATTEMPTS } };
        match fetch(&req, dest, counters.clone(), cancel, state_path).await {
            Ok(n) => return Ok(n),
            Err(e @ (AppError::Cancelled | AppError::DiskFull | AppError::Permission(_) | AppError::Io(_))) => return Err(e),
            Err(e) if more => {
                let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
                tracing::warn!(host, "media host failed, trying its mirror: {e}");
                last = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| AppError::Network("no host could serve the file".into())))
}

/// How many segments may hold a connection at once. Starts at the segment count and shrinks
/// when the server pushes back (503/429, or refusing/resetting a connection before sending any
/// data): some CDNs cap parallel connections per client, and retrying at the same width only
/// repeats the refusal. It never grows back during a transfer.
pub struct Gate {
    state: Mutex<(usize, usize)>, // (active, limit)
    freed: tokio::sync::Notify,
}

impl Gate {
    pub fn new(limit: usize) -> Self {
        Gate { state: Mutex::new((0, limit.max(1))), freed: tokio::sync::Notify::new() }
    }

    pub fn limit(&self) -> usize {
        self.state.lock().unwrap().1
    }

    async fn acquire(&self, cancel: &CancellationToken) -> AppResult<()> {
        loop {
            let freed = self.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            {
                let mut s = self.state.lock().unwrap();
                if s.0 < s.1 {
                    s.0 += 1;
                    return Ok(());
                }
            }
            tokio::select! {
                _ = freed => {}
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
            }
        }
    }

    fn release(&self) {
        self.state.lock().unwrap().0 -= 1;
        self.freed.notify_waiters();
    }

    fn shrink(&self) {
        let mut s = self.state.lock().unwrap();
        if s.1 > 1 {
            s.1 -= 1;
            tracing::info!(limit = s.1, "server pushed back; using fewer parallel connections");
        }
    }
}

/// Errors that mean "too many connections", as opposed to a broken transfer.
fn is_pushback(e: &AppError, received: bool) -> bool {
    matches!(e, AppError::Http(429 | 503)) || (!received && matches!(e, AppError::Network(_)))
}

#[allow(clippy::too_many_arguments)]
async fn run_segment(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    dest: &Path,
    seg: Arc<Segment>,
    mut initial: Option<Response>,
    ranged: bool,
    counters: &Counters,
    gate: &Gate,
    max_attempts: u32,
    cancel: &CancellationToken,
) -> AppResult<()> {
    let mut attempt = 0u32;
    loop {
        gate.acquire(cancel).await?;
        let before = seg.done.load(Ordering::Relaxed);
        let result = segment_attempt(client, url, &headers, dest, &seg, initial.take(), ranged, counters, cancel).await;
        gate.release();
        match result {
            Ok(()) => return Ok(()),
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(e @ (AppError::DiskFull | AppError::Permission(_) | AppError::Io(_) | AppError::Expired(_))) => return Err(e),
            Err(AppError::Http(c)) if c != 429 && !(500..=599).contains(&c) => return Err(AppError::Http(c)),
            Err(e) => {
                let received = seg.done.load(Ordering::Relaxed) > before;
                if is_pushback(&e, received) {
                    gate.shrink();
                }
                // Real progress since the last failure earns a fresh set of attempts. A host that cuts
                // every connection after a few hundred KB (observed on a CDN mirror) doesn't, so it is
                // given up on and its mirror used instead.
                let progressed = seg.done.load(Ordering::Relaxed) - before >= PROGRESS_RESETS_RETRIES;
                attempt = if progressed { 1 } else { attempt + 1 };
                if attempt > max_attempts {
                    return Err(e);
                }
                if !ranged {
                    // The server can't resume: start this (only) segment over.
                    let had = seg.done.swap(0, Ordering::Relaxed);
                    seg.persisted.store(0, Ordering::Relaxed);
                    counters.done.fetch_sub(had, Ordering::Relaxed);
                }
                tracing::warn!(attempt, error = %e, "segment retry");
                sleep_or_cancel(backoff(attempt), cancel).await?;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn segment_attempt(
    client: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    dest: &Path,
    seg: &Segment,
    initial: Option<Response>,
    ranged: bool,
    counters: &Counters,
    cancel: &CancellationToken,
) -> AppResult<()> {
    let done = seg.done.load(Ordering::Relaxed);
    if seg.len != u64::MAX && done >= seg.len {
        return Ok(());
    }
    let resp = match initial {
        Some(r) => r,
        None => {
            let mut rb = client.get(url).headers(headers.clone());
            if ranged {
                let from = seg.start + done;
                let range = if seg.len == u64::MAX { format!("bytes={from}-") } else { format!("bytes={from}-{}", seg.start + seg.len - 1) };
                rb = rb.header(RANGE, range);
            }
            let r = tokio::select! {
                r = rb.send() => r?,
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
            };
            if r.status() == StatusCode::TOO_MANY_REQUESTS {
                if let Some(w) = retry_after(&r) {
                    sleep_or_cancel(w, cancel).await?;
                }
                return Err(AppError::Http(429));
            }
            if !r.status().is_success() {
                return Err(status_error(&r));
            }
            if ranged && r.status() != StatusCode::PARTIAL_CONTENT {
                return Err(AppError::Network("the server stopped honouring byte ranges".into()));
            }
            r
        }
    };

    let mut file = tokio::fs::OpenOptions::new().write(true).open(dest).await?;
    file.seek(std::io::SeekFrom::Start(seg.start + seg.done.load(Ordering::Relaxed))).await?;
    let mut w = BufWriter::with_capacity(WRITE_BUFFER, file);
    let mut stream = resp.bytes_stream();

    let outcome: AppResult<()> = async {
        loop {
            let next = tokio::select! {
                n = tokio::time::timeout(STALL_TIMEOUT, stream.next()) => n,
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
            };
            let chunk = match next {
                Err(_) => return Err(AppError::Network("the transfer stalled for 25 s".into())),
                Ok(None) => break,
                Ok(Some(Err(e))) => return Err(e.into()),
                Ok(Some(Ok(c))) => c,
            };
            let have = seg.done.load(Ordering::Relaxed);
            let remaining = seg.len.saturating_sub(have);
            let take = (chunk.len() as u64).min(remaining) as usize;
            w.write_all(&chunk[..take]).await?;
            seg.done.fetch_add(take as u64, Ordering::Relaxed);
            counters.done.fetch_add(take as u64, Ordering::Relaxed);
            // Everything not still sitting in the write buffer has been handed to the OS and
            // survives a crash of Rabbit.
            seg.persisted.store(seg.done.load(Ordering::Relaxed) - w.buffer().len() as u64, Ordering::Relaxed);
            if take < chunk.len() || (seg.len != u64::MAX && seg.done.load(Ordering::Relaxed) >= seg.len) {
                break; // segment satisfied (the probe response runs past it)
            }
        }
        Ok(())
    }
    .await;
    w.flush().await?;
    seg.persisted.store(seg.done.load(Ordering::Relaxed), Ordering::Relaxed);
    outcome?;
    if seg.len != u64::MAX && seg.done.load(Ordering::Relaxed) < seg.len {
        return Err(AppError::Network("the connection closed early".into()));
    }
    Ok(())
}
