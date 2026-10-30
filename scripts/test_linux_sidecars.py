"""Offline checks for the Linux download, checksum and packaging preparation path."""

import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("sidecars", Path(__file__).with_name("fetch-sidecars-linux.py"))
sidecars = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sidecars)


class LinuxSidecars(unittest.TestCase):
    def test_checksum_failure_leaves_no_binary_or_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "yt-dlp"
            with patch.object(sidecars, "read", return_value=io.BytesIO(b"tampered")):
                with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                    sidecars.verified("https://example.invalid", "yt-dlp_linux", "0" * 64 + "  yt-dlp_linux", dest)
            self.assertEqual(list(Path(tmp).iterdir()), [])

    def test_native_package_preparation_and_executable_permissions(self):
        data = io.BytesIO()
        with tarfile.open(fileobj=data, mode="w:xz") as tar:
            for name, body in [("bin/ffmpeg", b"ffmpeg"), ("bin/ffprobe", b"ffprobe"), ("LICENSE.txt", b"LGPL")]:
                info = tarfile.TarInfo("ffmpeg/" + name)
                info.size = len(body)
                tar.addfile(info, io.BytesIO(body))
        archive = data.getvalue()
        ff_asset = "ffmpeg-n9.0.2-33-g4cf298e66a-linux64-lgpl-9.0.tar.xz"
        ff_base = f"https://github.com/BtbN/FFmpeg-Builds/releases/download/{sidecars.FF_TAG}"
        yt_base = f"https://github.com/yt-dlp/yt-dlp/releases/download/{sidecars.YT_VERSION}"
        routes = {
            f"{yt_base}/yt-dlp_linux": b"yt-dlp",
            f"{yt_base}/SHA2-256SUMS": (hashlib.sha256(b"yt-dlp").hexdigest() + "  yt-dlp_linux").encode(),
            f"{ff_base}/{ff_asset}": archive,
            f"{ff_base}/checksums.sha256": (hashlib.sha256(archive).hexdigest() + "  " + ff_asset).encode(),
            f"https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/tags/{sidecars.FF_TAG}": json.dumps({"assets": [{"name": ff_asset}]}).encode(),
        }
        with tempfile.TemporaryDirectory() as tmp:
            with patch.object(sidecars.platform, "machine", return_value="x86_64"), \
                 patch.object(sidecars.platform, "system", return_value="Linux"), \
                 patch.object(sidecars, "read", side_effect=lambda url: io.BytesIO(routes[url])), \
                 patch("sys.argv", ["fetch-sidecars-linux.py", "--output", tmp]):
                sidecars.main()
            for name in ["yt-dlp", "ffmpeg", "ffprobe"]:
                self.assertEqual((Path(tmp) / name).stat().st_mode & 0o777, 0o755)
            self.assertEqual((Path(tmp) / "ffprobe").read_bytes(), b"ffprobe")
            self.assertEqual((Path(tmp) / "FFmpeg-LICENSE.txt").read_bytes(), b"LGPL")
            self.assertIn(sidecars.FF_TAG, (Path(tmp) / "FFmpeg-SOURCE.txt").read_text())


if __name__ == "__main__":
    unittest.main()
