<div align="center">

<img src="src-tauri/icons/128x128.png" width="88" height="88" alt="Rabbit logo">

# Rabbit

**Paste a link. Get the real file.**

A fast, private media downloader for Linux. Rabbit downloads public videos, audio and images from YouTube,
Bilibili, Kuaishou, RedNote, Instagram, Facebook, X, Pinterest and more — in the best quality the source offers —
and checks every file it saves.

[![Latest release](https://img.shields.io/github/v/release/aadiichau/rabbit-linux?label=release&color=e5a23c)](https://github.com/aadiichau/rabbit-linux/releases/latest)
[![CI](https://github.com/aadiichau/rabbit-linux/actions/workflows/ci.yml/badge.svg)](https://github.com/aadiichau/rabbit-linux/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-2f333a)](LICENSE)

### [⬇ Download for Linux](https://github.com/aadiichau/rabbit-linux/releases/latest) · [Windows app](https://github.com/aadiichau/rabbit) · [Changelog](CHANGELOG.md)

<sub>Linux x86_64 · AppImage, Debian and RPM packages · same Rabbit interface and download engine</sub>

<img src="site/assets/rabbit-analyze-4k.webp" width="900" alt="Rabbit analyzing a YouTube link: twelve video options from 2160p60 to 144p with container, codec, frame rate and size, and 1080p60 H.264 recommended.">

</div>

## Why Rabbit

- **Every option, laid out.** Resolution, frame rate, container, codec and size for every format the source
  offers. Rabbit recommends the one that plays everywhere; you can pick any other.
- **Verified files.** After each download Rabbit inspects the file with FFprobe. If a video comes out smaller than
  the quality it promised, the task fails instead of quietly saving the wrong thing.
- **Fast, resumable transfers.** Large files are fetched over parallel connections, back off politely when a
  server limits them, and resume after a pause, a dropped connection or a restart.
- **Its own extractors.** Native readers for Bilibili, Kuaishou, Kwai, RedNote, X, Pinterest and Suno, with
  [yt-dlp](https://github.com/yt-dlp/yt-dlp) as the fallback and for everything else.
- **Everything included.** yt-dlp, FFmpeg and FFprobe ship inside the installer: merge video and audio, convert to
  MP3 or save the original image with nothing else to install. yt-dlp can be updated from Settings → Tools.
- **Private by default.** No account, no telemetry, no ads. Links are analysed on your PC. Signed-in sessions
  (for Bilibili HD) are opt-in, per site, and cookie values are never stored or logged.

<p align="center">
  <img src="site/assets/rabbit-queue.webp" width="900" alt="Rabbit's queue: two Blender open movies downloading with size, speed and time left, one finished and marked Verified.">
</p>

## Supported platforms

Each status comes from downloading real public samples and checking them with FFprobe
([full details](docs/PLATFORM_COMPATIBILITY.md)). Where a site withholds something from signed-out visitors,
Rabbit says so — it doesn't route around it.

| Platform | Video | Audio | Images | Best verified | Status |
|---|---|---|---|---|---|
| YouTube | Videos, Shorts | M4A, Opus, MP3 | Cover | 3840×2160 | ✅ Verified |
| Instagram | Reels, posts | — | Photos, carousels | 1080×1920 | ✅ Verified |
| X / Twitter | Video, GIF | — | Original photos | 1080×1440 | ✅ Verified |
| Bilibili | BV / av, parts, b23.tv | M4A, MP3 | Cover | 1920×1080 ¹ | ✅ Verified |
| RedNote | Video notes | MP3 | Image notes, Live Photos | 720×1280 | ✅ Verified |
| Pinterest | Video pins | — | Image and story pins | 720×1280 | ✅ Verified |
| Kwai | Video pages | MP3 | Cover | 720×1280 | ✅ Verified |
| Kuaishou | Share and desktop links | MP3 | Cover | 1080×1920 | 🟡 Partial ² |
| Facebook | Reels, videos | — | — | 1080×1920 | 🟡 Partial ³ |
| Suno | Song video | Withheld by Suno | Cover | 824×1464 | 🟡 Partial |
| TikTok, Douyin | — | — | — | — | Not tested ⁴ |

¹ Signed-out visitors get up to 480p; 1080p was verified with the optional signed-in session.
² Photo albums untested (no public sample). ³ Photos not supported; some pages send no video to signed-out
visitors. ⁴ Blocked at the test location (regional ban); Douyin also needs a signed-in session.
Other sites go through yt-dlp's generic support, with no promise.

## Install

Download the Linux package from [Releases](https://github.com/aadiichau/rabbit-linux/releases/latest):

| Download | For |
|---|---|
| `Rabbit_0.7.1_amd64.AppImage` | **CachyOS, Arch and other x86_64 Linux desktops.** Make it executable and run it. |
| `Rabbit_0.7.1_amd64.deb` | Ubuntu 22.04+ and compatible Debian-based distributions. |
| `Rabbit-0.7.1-1.x86_64.rpm` | Compatible RPM-based distributions. |
| `SHA256SUMS-linux.txt` | SHA-256 checksums for all release downloads. |

For CachyOS / Arch:

```sh
chmod +x Rabbit_0.7.1_amd64.AppImage
./Rabbit_0.7.1_amd64.AppImage
```

If FUSE is unavailable, run `./Rabbit_0.7.1_amd64.AppImage --appimage-extract-and-run`.
Check downloads with `sha256sum --check SHA256SUMS-linux.txt` from the folder containing the packages.

The packages include Linux yt-dlp, FFmpeg and FFprobe. Keep Node.js 24+ or Deno available for
YouTube JavaScript challenges. GTK3 and WebKitGTK 4.1 are required for native packages; AppImage bundles
its GUI libraries. Packages are built on Ubuntu 22.04; a glibc-based Linux desktop is required.
Settings, library and tool updates live under `~/.local/share/com.rabbit.app` (or `$XDG_DATA_HOME`).
Downloaded files are never deleted by uninstalling Rabbit.

See [Linux setup and build instructions](docs/LINUX.md) and [test results](docs/LINUX_TEST_RESULTS.md).
For Windows installers, use the [original Rabbit repository](https://github.com/aadiichau/rabbit).

## FAQ

<details><summary><b>Is Rabbit free?</b></summary>

Yes — open source under the [MIT License](LICENSE), with no ads, accounts or paid tiers.
</details>

<details><summary><b>Does Rabbit collect any data?</b></summary>

No. There is no telemetry or analytics. Rabbit connects only to the site you're downloading from — and, when you
ask it to, GitHub to check for a yt-dlp update. Your library and history stay in a local database on your PC.
</details>

<details><summary><b>Can it download private posts or get around paywalls?</b></summary>

No. Rabbit downloads what a site shows a signed-out visitor and never bypasses DRM, paywalls, captchas or regional
blocks. For Bilibili you can opt in to your own Firefox session or a `cookies.txt` export to get the qualities
your account already has ([design notes](docs/DESIGN_SIGNED_IN_SESSIONS.md)).
</details>

<details><summary><b>Does Rabbit update itself?</b></summary>

No. Download new versions from the [Releases page](https://github.com/aadiichau/rabbit-linux/releases). The bundled
yt-dlp can be updated from Settings → Tools when you choose; each download is checked against its published
SHA-256 first.
</details>

<details><summary><b>Is this the same app as Rabbit for Windows?</b></summary>

Yes. This Linux port shares the interface, download engine, extractors, library and settings with
[the original Rabbit](https://github.com/aadiichau/rabbit). Platform-specific code handles Linux tools,
process cancellation, Firefox profiles and desktop integration. No macOS build is provided.
</details>

<details><summary><b>A link doesn't work. What should I do?</b></summary>

Sites change often. Update yt-dlp in Settings → Tools and try again. If it still fails,
[open an issue](https://github.com/aadiichau/rabbit-linux/issues/new/choose) with the message Rabbit shows. Please don't
post links to private content or anything that identifies you.
</details>

<details><summary><b>Is downloading allowed?</b></summary>

Only download what you have the right to. Many sites' terms restrict downloading, and copyright applies to what you
save. Rabbit is a tool; how you use it is your responsibility.
</details>

## Build from source

Install Rust, Node.js 24+, Python 3 and the [Linux build prerequisites](docs/LINUX.md), then:

```sh
git clone https://github.com/aadiichau/rabbit-linux.git
cd rabbit-linux
python3 scripts/fetch-sidecars-linux.py  # pinned native tools, SHA-256 verified
npm ci
npm run tauri dev
# Build a Linux package:
node scripts/third-party-notices.mjs
npm run tauri build -- --bundles appimage
```

Tests: `npm test` (frontend), `cargo test` in `src-tauri` (offline suites). Live-network tests are marked
`#[ignore]` and run with `cargo test -- --ignored`. See [CONTRIBUTING.md](CONTRIBUTING.md).

Documentation: [architecture](docs/ARCHITECTURE.md) · [extractors](docs/EXTRACTOR_ARCHITECTURE.md) ·
[design system](docs/DESIGN_SYSTEM.md) · [quality verification](docs/QUALITY_VERIFICATION.md) ·
[test results](docs/TEST_RESULTS.md).

## Credits and licence

Rabbit is released under the [MIT License](LICENSE). It bundles [yt-dlp](https://github.com/yt-dlp/yt-dlp)
(Unlicense) and [FFmpeg](https://ffmpeg.org/) (LGPL-3.0-or-later, static Linux build by
[BtbN](https://github.com/BtbN/FFmpeg-Builds)), which run as separate programs; their licences and the FFmpeg
source pointer are installed under the app resources (`licenses/` and `tools/`). Built with [Tauri](https://tauri.app/), React and IBM Plex (OFL).
All dependencies: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Screenshots show Big Buck Bunny, Sintel, Spring and Tears of Steel by the Blender Foundation (CC BY) and a photo by
NASA.
