#!/usr/bin/env python3
"""Prepare checksum-verified, native Linux tools without installing system packages."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import tarfile
import tempfile
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]
YT_VERSION = "2026.10.21"
FF_TAG = "autobuild-2026-10-22-13-06"


def read(url):
    headers = {"User-Agent": "rabbit-linux-build", "Accept": "application/vnd.github+json"}
    if url.startswith("https://api.github.com/") and os.environ.get("GITHUB_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GITHUB_TOKEN"]
    return urlopen(Request(url, headers=headers), timeout=120)


def verified(base, asset, sums, dest):
    expected = next((parts[0].lower() for line in sums.splitlines()
                     if len(parts := line.split()) == 2
                     and parts[1].lstrip("*") == asset
                     and re.fullmatch(r"[a-fA-F0-9]{64}", parts[0])), None)
    if expected is None:
        raise RuntimeError(f"No SHA-256 published for {asset}")
    if dest.is_file() and hashlib.sha256(dest.read_bytes()).hexdigest() == expected:
        return
    tmp = dest.with_name(dest.name + ".download")
    try:
        digest = hashlib.sha256()
        with read(f"{base}/{asset}") as src, tmp.open("wb") as out:
            while chunk := src.read(1024 * 1024):
                digest.update(chunk)
                out.write(chunk)
        if digest.hexdigest() != expected:
            raise RuntimeError(f"SHA-256 mismatch for {asset}")
        tmp.replace(dest)
    finally:
        tmp.unlink(missing_ok=True)
    print(f"Verified {asset}: {expected}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "src-tauri/binaries/linux")
    args = parser.parse_args()
    arches = {"x86_64": ("yt-dlp_linux", "linux64"),
              "aarch64": ("yt-dlp_linux_aarch64", "linuxarm64")}
    if platform.system() != "Linux" or platform.machine() not in arches:
        parser.error("run on Linux x86_64 or aarch64; cross-compilation is not supported")
    yt_asset, ff_arch = arches[platform.machine()]
    dest = args.output
    dest.mkdir(parents=True, exist_ok=True)
    yt_base = f"https://github.com/yt-dlp/yt-dlp/releases/download/{YT_VERSION}"
    with read(f"{yt_base}/SHA2-256SUMS") as response:
        verified(yt_base, yt_asset, response.read().decode(), dest / "yt-dlp")

    # Pick the same pinned FFmpeg release branch as Windows, with static LGPL libraries.
    with read(f"https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/{FF_TAG}") as response:
        release = json.load(response)
    assets = [a["name"] for a in release["assets"]
              if re.fullmatch(rf"ffmpeg-n9\..*-{ff_arch}-lgpl-9\.0\.tar\.xz", a["name"])]
    if len(assets) != 1:
        raise RuntimeError(f"Expected one LGPL FFmpeg 9.0 asset for {ff_arch}, found {assets}")
    asset = assets[0]
    ff_base = f"https://github.com/BtbN/FFmpeg-Builds/releases/download/{FF_TAG}"
    with read(f"{ff_base}/checksums.sha256") as response:
        sums = response.read().decode()
    with tempfile.TemporaryDirectory(prefix="rabbit-ffmpeg-") as tmp:
        archive = Path(tmp) / asset
        verified(ff_base, asset, sums, archive)
        with tarfile.open(archive) as tar:
            # Copy only regular binary/license files; never extract arbitrary archive paths.
            for name, suffix in [("ffmpeg", "/bin/ffmpeg"), ("ffprobe", "/bin/ffprobe"),
                                 ("FFmpeg-LICENSE.txt", "/LICENSE.txt")]:
                members = [m for m in tar.getmembers() if m.isfile() and m.name.endswith(suffix)]
                if len(members) != 1:
                    raise RuntimeError(f"Missing or ambiguous {suffix} in {asset}")
                with tar.extractfile(members[0]) as src, (dest / name).open("wb") as out:
                    shutil.copyfileobj(src, out)
    (dest / "FFmpeg-SOURCE.txt").write_text(
        f"{asset}\n{ff_base}/{asset}\nhttps://github.com/BtbN/FFmpeg-Builds/tree/{FF_TAG}\n"
        "LGPL static build; build recipes and corresponding source references are at the URL above.\n")
    for tool in ("yt-dlp", "ffmpeg", "ffprobe"):
        (dest / tool).chmod(0o755)
    print(f"Linux tools ready in {dest}")


if __name__ == "__main__":
    main()
