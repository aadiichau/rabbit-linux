# Security policy

## Supported versions

Only the latest release receives fixes. Please update before reporting.

## Reporting a vulnerability

Please **don't open a public issue** for security problems. Use GitHub's private reporting instead:
[Report a vulnerability](https://github.com/aadiichau/rabbit-linux/security/advisories/new).

Include what you found, how to reproduce it, and the Rabbit version. Fixes are released as a new version, with
credit to you unless you prefer otherwise.

## What Rabbit does with your data

- No telemetry or analytics. Rabbit connects to the sites you download from, and to GitHub only when you check
  for a yt-dlp update.
- Signed-in sessions are off by default and enabled per site. Cookie values are read when needed, sent only to
  that site's own hosts (or to yt-dlp through a temporary file deleted after each run), and never stored or logged.
- yt-dlp updates are verified against the release's published SHA-256 before use.

## Installer integrity

Linux release packages are built by GitHub Actions from the release source. Each release includes
`SHA256SUMS-linux.txt`; check it against the files you download. Download Rabbit Linux only from
this repository's Releases page.
