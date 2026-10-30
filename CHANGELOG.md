# Changelog

## 0.7.1 — 2026-10-30 · First Linux release

- Native Linux port of Rabbit with the same React interface, extractors, queue, library and settings.
- Linux x86_64 AppImage, Debian and RPM packaging with bundled checksum-verified yt-dlp, FFmpeg and FFprobe.
- Unix process groups stop the full download/conversion tree on cancellation.
- Firefox session discovery supports standard, Flatpak and Snap profiles, with private temporary cookie files.
- Settings → Tools installs the correct Linux yt-dlp asset and preserves checksum/version verification.
- Correct Linux sparse-file handling and automatic WebKit renderer fallback for NVIDIA/X11 blank windows.
- Platform-neutral notification and file-manager labels.
- Linux CI checks frontend/backend tests and builds packages on Ubuntu 22.04.
- Tested on CachyOS: 100 automated checks passed, plus a real GUI video download and restart persistence.


## 0.7.0 — 2026-10-24 · First public release; signed-in sessions (opt-in)

**First public release.** Windows installer `Rabbit_0.7.0_x64-setup.exe` (Windows 10 / 11, 64-bit):

- Installs per user to `%LOCALAPPDATA%\Rabbit` without administrator rights, with a Start menu entry and an
  uninstaller (Settings → Apps). Reinstalling or upgrading keeps your library and settings.
- **Portable version** `Rabbit_0.7.0_x64-portable.zip`: unzip and run `rabbit.exe`, no installation. A
  `portable.txt` next to the exe keeps the library, settings, yt-dlp updates and WebView2 data in a `data` folder
  beside it, so the folder can move between PCs or run from a USB drive. Needs WebView2 (included in Windows 11).
- Bundles yt-dlp 2026.10.21, FFmpeg and FFprobe n9.0.2 (LGPL shared build) and their licences under `licenses\`.
  The WebView2 bootstrapper is embedded and installs WebView2 if it's missing (Windows 10).
- **Not code-signed:** Windows SmartScreen may warn about an unknown publisher. Each release has a
  `SHA256SUMS.txt`. Rabbit doesn't update itself; new versions are published on GitHub Releases.
- Quality labels follow the standard tier of the frame: a 1920×800 film is "1080p" (not "800p"), a 1080×1920
  portrait video is "1080p".
- Open source under the MIT License (`LICENSE`, `THIRD_PARTY_NOTICES.md`).

Known limitations: Windows only; TikTok and Douyin not tested (regionally blocked at the test location); Facebook
photos and Kuaishou photo albums not supported; Suno audio withheld by Suno; Bilibili above 480p needs the opt-in
signed-in session.

- **Settings → Sessions → Signed-in sites** (off by default): use your own browser session for Bilibili (HD up
  to what your account allows, instead of the guest 480p) and Douyin (videos aren't shown to signed-out
  visitors). Source: a Firefox profile or an exported cookies.txt. Chrome/Edge cookies can't be read (App-Bound
  Encryption) and Rabbit doesn't try.
- Only the switched-on site's cookies are read, values are never stored or logged, Bilibili's go only to
  api.bilibili.com / www.bilibili.com, Douyin's go to yt-dlp in a temporary file deleted after each run. A
  "Check" button shows whether the session is signed in (and, for Bilibili, the account and tier).
- The analysis line says when your session was used, or why Rabbit fell back to guest qualities.
- **Verified with a real signed-in account:** Bilibili 1080p (1920×1080 H.264 + AAC). Douyin with a session
  can't be tested from India, where Douyin is banned.
- **Mirror failover:** native downloads carry the source's backup URLs (Bilibili, Kuaishou, RedNote) and switch
  hosts when one keeps failing. Found with the signed-in test: Bilibili's Akamai host cut HD streams after
  ~270 KB, while its bilivideo.com backup served them in full. Only ≥ 2 MiB of progress now resets a segment's
  retry budget, so a host that keeps cutting connections short is abandoned instead of milked.
- Messages no longer claim Rabbit "doesn't sign in" where the opt-in now exists; Rabbit still never asks for
  passwords.
- Tests: 67 unit (+ cookie sources), session host allow-list against a recording server, live fallback test with
  made-up cookies, and two live tests gated on your own session (`RABBIT_SESSION_SOURCE`).

## 0.6.0 — 2026-10-23 · Platform compatibility and quality

- **New platforms and media types:** X photo posts, mixed posts and GIFs (from X's public embed data, original
  photos up to 4096 px); Instagram photo posts and photo carousels; Pinterest image pins and multi-page story
  pins; RedNote image notes with Live Photos (full-resolution JPEG, e.g. 3193×5744) and xhslink.cn links; Kwai
  videos; Suno songs (the public song video and cover — Suno withholds the audio file from signed-out visitors).
- **Quality verification:** every finished video is checked against the resolution its option promised and fails
  if it comes out clearly smaller. This caught Pinterest renditions labelled 1080×1920 that are 720×1280; sizes
  are now corrected. X GIFs no longer borrow the display size.
- **Verified top qualities:** YouTube 2160p60, YouTube Shorts 1080p60, Instagram/Facebook/Kuaishou 1080p, X
  1080×1440 (docs/QUALITY_VERIFICATION.md).
- **Transfer engine:** segments back off when a server limits parallel connections (503/429/refused
  connections), instead of failing; progress resets a segment's retry budget. Default connections per file 8 → 16
  and minimum segment 2 → 4 MiB, from measurements: YouTube 6.3 → 12.4 MB/s; small files no longer over-split.
  (Existing installs keep their saved setting.)
- Extractor HTTP requests retry once after a connection error; Kuaishou desktop links skip the flaky
  www.kuaishou.com host.
- Gallery sizes are no longer shown as a misleading partial estimate when some items have unknown sizes.
- Documentation: PLATFORM_COMPATIBILITY, EXTRACTOR_ARCHITECTURE, EXTRACTION_FAILURES, QUALITY_VERIFICATION,
  TEST_RESULTS.
- Tests: 61 unit, 11 engine (+ connection cap), 58-URL platform matrix with per-file FFprobe data, transfer
  benchmark (`tests/bench_transfer.rs`).

## 0.5.0 — 2026-10-23 · Bilibili, Kuaishou and RedNote

- **Bilibili** works again: Rabbit reads ordinary video pages (BV / av links, `?p=` parts of multi-part videos,
  b23.tv / bili2233.cn / bili.im short links) through the public web API bilibili.com's own player uses, so it no
  longer depends on yt-dlp's broken extractor (yt-dlp issue #17894). Video (480p and 360p for signed-out
  visitors), original M4A audio, MP3 and cover. yt-dlp remains the fallback and the route for other Bilibili pages.
- **Kuaishou** is supported for videos: v.kuaishou.com share links, www.kuaishou.com/short-video and /f/ links,
  through Kuaishou's public share page, with automatic retry on another share host when one times out.
  Photo albums and kwai.com are declined with a clear reason.
- **RedNote** video notes work from share links (xhslink.com, or note links carrying `xsec_token`), through the
  public mobile note page on rednote.com — which also avoids xiaohongshu.com's connection resets on some
  networks. Image notes, galleries and Live Photos are implemented but not yet tested on a real note.
- Extraction chain: Rabbit's own extractor first where one exists, validated by the normalizer, then yt-dlp.
  When both fail, the platform's actual reason is shown (expired share link, missing share token, removed
  video, region lock, sign-in only).
- Portrait videos are labelled by their short side (a 720×1280 video is "720p", not "1280p"), and the
  recommendation treats 1080×1920 as 1080p.
- Clearer message when a yt-dlp extractor crashes internally (site changed).
- **FFmpeg and FFprobe are bundled** (BtbN LGPLv3 shared build n9.0.2, pinned and SHA-256 verified by
  `scripts/fetch-sidecars.ps1`; licence and source pointer installed under `licenses/`). Release builds can now
  merge video + audio (Bilibili, YouTube) and convert to MP3 with nothing else installed.
- New dependency: `md-5` (Bilibili's request signature).
- Tests: 45 unit (+ WBI signing against the published reference values, page-state parsing, address
  recognition), `tests/native_platforms.rs` (7 live tests incl. pause → restart → resume through the native
  extractor), 8 new matrix URLs.

## 0.4.0 — 2026-10-23 · yt-dlp updater

- Settings → Tools: check for and install yt-dlp updates from the official Stable or Nightly releases, with live
  download progress. Each download is SHA-256 verified against the release's checksum file and must report the
  expected version before it is used; failures install nothing. "Use bundled version" reverts.
- Updates apply to the next analysis or download without restarting Rabbit; Rabbit never checks on its own.

## 0.3.0 — 2026-10-23 · Platform compatibility (Phase 6)

- Platform probe: 35 public URLs across 10 platforms through the real pipeline; results and evidence in
  FEATURE_MATRIX. Verified: YouTube, Pinterest, X (videos + multi-video posts), Instagram reels, Facebook reels.
- Silent videos (no audio track) are now downloadable; unknown audio is no longer assumed or verified against.
- Generic-extractor pages are no longer mistaken for videos.
- Honest errors: posts that only link elsewhere, image-only posts, regional blocks (TikTok), sign-in requirements
  (Douyin), connection resets (RedNote), and an upstream yt-dlp break (Bilibili, issue #17894).
- Suno and Kuaishou are declined up front with the reason.

## 0.2.0 — 2026-10-22 · Download manager (Phase 5)

- Pause / resume / cancel with resume-state files; only OS-persisted bytes are recorded; validated by size + ETag.
- Queue survives restarts and crashes: running tasks return as "Interrupted" with saved bytes; resuming
  re-analyzes the source (stream URLs are never written to disk) and continues from the partial files.
- Expired links mid-download trigger one automatic re-analysis.
- Pause all / Resume all; keyboard control of queue rows (Space, Delete, Enter).
- Windows notifications on finish/failure while Rabbit is in the background (setting, on by default).
- Library: platform and type filter chips with counts; sort by newest, oldest, largest, title.
- Embedded application manifest (Common Controls v6, PerMonitorV2 DPI, long paths) for app and tests.
- Tests: 21 unit, 10 engine, 4 live end-to-end (incl. mid-file restart recovery).

## 0.1.0 — 2026-10-22 · Functional MVP (Phase 4)

- Research: DLBunny investigated with REA (5 CLI captures, evidence in `research/rea/`); docs for research,
  feature matrix, architecture, design system, plan, tests, benchmarks.
- App: Tauri 2 shell with custom title bar, Home (paste/drag/Ctrl+V, local platform detection, analysis with
  live timer), media preview with real format table, Queue with ruler progress and segment bars, Library
  (SQLite), Settings.
- Engine: yt-dlp extraction (sandboxed in Job Objects, no config/plugins), format normaliser, native
  segmented HTTP downloader with resume-on-drop and backoff, yt-dlp-managed HLS/DASH fragments, FFmpeg
  stream-copy merge, MP3 conversion, ffprobe verification, collision-safe naming.
- Tests: 19 unit, 7 engine fault-injection, 2 live end-to-end, 4 TypeScript.
