# Contributing to Rabbit

Thanks for helping. Bug reports, platform fixes and focused pull requests are all welcome.

## Before you start

- **Bugs:** open an issue with the bug template. Include the platform, the message Rabbit shows, the Rabbit and
  yt-dlp versions (Settings → Tools) and, if you can, a *public* link that reproduces it. Never post links to
  private content, cookies, or anything that identifies you.
- **Larger changes** (a new platform, a new setting, UI changes): open an issue first so we can agree on the
  approach.
- **Out of scope:** anything that bypasses DRM, paywalls, captchas, regional blocks or a site's sign-in. Rabbit
  only downloads what a site already serves to the visitor.

## Development setup

Windows 10/11 x64, Rust stable (MSVC), Node.js 24 and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/).

```powershell
./scripts/fetch-sidecars.ps1   # yt-dlp + FFmpeg, pinned and SHA-256 verified
npm ci
npm run tauri dev
```

## Checks (the same ones CI runs)

```powershell
npx tsc --noEmit
npx vitest run
npm run build
cd src-tauri
cargo clippy --all-targets --locked
cargo test --locked
```

Live-network tests are `#[ignore]` and need internet access: `cargo test -- --ignored`. Tests that need a
signed-in session read it from `RABBIT_SESSION_SOURCE` and are skipped without it.

## Pull requests

- Keep each PR to one change, and match the surrounding code's style.
- Add or update tests for behaviour changes; for a platform fix, add a public sample to
  `src-tauri/tests/platform-urls.json` when one exists.
- Update `CHANGELOG.md` and the docs your change affects (`docs/PLATFORM_COMPATIBILITY.md` for platform support).
- Don't commit downloaded media, cookies, databases, logs or personal links.
- New dependencies need a reason and a permissive licence; run `node scripts/third-party-notices.mjs` to refresh
  `THIRD_PARTY_NOTICES.md`.

By contributing you agree that your contribution is licensed under the MIT License.

## Releases (maintainers)

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`, and add a
   `CHANGELOG.md` entry.
2. Push a tag `vX.Y.Z`. The Release workflow checks that the versions match, builds the installer, and creates a
   **draft** release with the installer and `SHA256SUMS.txt`.
3. Install and test the draft's installer, paste the changelog entry into the release notes, then publish it.
   The website and download counter update automatically.

## Linux

See [Linux development and packaging](docs/LINUX.md). Linux changes share the existing UI and engine;
keep OS-specific dependencies behind target guards and preserve Windows behavior.
