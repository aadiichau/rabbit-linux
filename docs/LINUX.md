# Rabbit on Linux

The Linux port shares the Windows interface, extractors, download engine, library, settings,
verification, notifications and signed-in sessions. Linux uses native processes and WebKitGTK.
Windows packaging remains available from the same source tree.

## Build and run

Use native Linux x86_64 or aarch64, stable Rust, Python 3 and Node.js 24.
Cross-compilation of the bundled tools is not supported.

On CachyOS / Arch, install build requirements:

```sh
sudo pacman -S --needed base-devel rust nodejs npm python webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg patchelf
```

On Ubuntu 22.04 or newer / Debian with WebKitGTK 4.1:

```sh
sudo apt-get install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev python3
```

With Rust and Node.js 24 installed:

```sh
python3 scripts/fetch-sidecars-linux.py
npm ci
npm run tauri dev
```

The preparation script downloads the pinned standalone Linux yt-dlp and static LGPL FFmpeg/FFprobe,
verifies upstream SHA-256 checksums before using them, and preserves FFmpeg's license/source references.
These private tools are packaged under Rabbit's resources; they do not replace system tools.
Linux can also use executable tools found on PATH. Keep Node.js 24 (or Deno) installed for
yt-dlp's JavaScript challenge handling on sites such as YouTube.

To produce a package:

```sh
node scripts/third-party-notices.mjs
npm run tauri build -- --bundles appimage
# Debian / Ubuntu:
npm run tauri build -- --bundles deb
# RPM distributions (requires rpmbuild):
npm run tauri build -- --bundles rpm
```

Outputs are in `src-tauri/target/release/bundle/`. For CachyOS, use the AppImage:

```sh
chmod +x src-tauri/target/release/bundle/appimage/*.AppImage
src-tauri/target/release/bundle/appimage/*.AppImage
```

If FUSE is unavailable, launch with `--appimage-extract-and-run`. AppImages built on
a newer distribution may require a newer glibc; the Linux workflow builds on Ubuntu 22.04
for broader compatibility. Native packages require the distribution's GTK3 and WebKitGTK 4.1.

The **Linux** GitHub Actions workflow runs the tests and builds `.AppImage`, `.deb` and `.rpm`
artifacts with checksums. It can also be started manually; it does not publish releases.

## Desktop integration and data

- The existing custom title bar, window controls, clipboard and file dialogs use Tauri's Linux backends.
- Notifications use the desktop notification service. A notification daemon must be running.
- Settings, library and yt-dlp updates use Tauri's per-user XDG data directory (`$XDG_DATA_HOME/com.rabbit.app`,
  normally `~/.local/share/com.rabbit.app`). Downloads default to the system's Downloads directory under `Rabbit`.
- Firefox session discovery includes standard, Flatpak and Snap profile locations, plus
  `$XDG_CONFIG_HOME/mozilla/firefox`. Profiles inaccessible to the current user require a cookies.txt export.
- Pausing or cancelling a download terminates its Unix process group, including spawned FFmpeg workers.
- On NVIDIA/X11, Rabbit uses WebKit's fallback renderer to avoid blank windows. An explicitly set
  `WEBKIT_DISABLE_DMABUF_RENDERER` takes precedence.
- Settings → Tools downloads the matching Linux yt-dlp release, checks its checksum and version,
  grants executable permission, and activates it. Reverting restores the bundled copy.

## Validation

```sh
npm test
npm run build
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Tests include native process-tree cancellation, executable permissions, resource tool discovery,
Firefox profile parsing and a local server exercising verified yt-dlp update/revert behavior.
The updater integration tests require the pinned tools prepared by the script above.
Live site tests remain opt-in (`-- --ignored`) and depend on site availability.

Local validation on CachyOS (2026-10-30): **100 automated tests passed**, the native desktop
executable built, and a real download completed through the interface and appeared as Verified.
The app rendered on native Wayland and on NVIDIA/XWayland with its automatic compatibility fallback.
Full details and remaining packaging/platform checks: [Linux test results](LINUX_TEST_RESULTS.md).
