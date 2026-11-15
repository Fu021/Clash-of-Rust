"""Offline checks for release validation and the generated product website."""
import copy
from contextlib import redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build


class WebsiteBuildTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.releases = json.loads((build.SOURCE / "releases.json").read_text(encoding="utf-8"))

    def test_public_snapshot_has_all_nine_packages_and_checksums(self):
        for channel, prerelease in (("stable", False), ("preview", True)):
            with self.subTest(channel=channel):
                record = self.releases[channel]
                self.assertIs(build.validate_release(record, prerelease), record)
                self.assertEqual(len(record["assets"]), 18)

    def test_invalid_metadata_is_rejected(self):
        for malformed in (None, [], {}, {"draft": False}):
            with self.subTest(record=malformed), self.assertRaises(ValueError):
                build.validate_release(malformed, False)
        changes = (
            ("draft", True),
            ("draft", None),
            ("prerelease", True),
            ("tag_name", None),
            ("tag_name", "v1.0.0/../../other"),
            ("html_url", "https://invalid.example/release"),
            ("assets", None),
            ("assets", [None]),
            ("assets", [{}]),
        )
        for field, value in changes:
            record = copy.deepcopy(self.releases["stable"])
            record[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                build.validate_release(record, False)

    def test_missing_duplicate_or_unsafe_assets_are_rejected(self):
        record = self.releases["stable"]
        missing = copy.deepcopy(record)
        missing["assets"].pop()
        duplicate = copy.deepcopy(record)
        duplicate["assets"].append(duplicate["assets"][0])
        for candidate in (missing, duplicate):
            with self.assertRaises(ValueError):
                build.validate_release(candidate, False)
        for field, value in (
            ("state", "new"),
            ("size", 0),
            ("size", True),
            ("browser_download_url", "https://invalid.example/installer"),
        ):
            candidate = copy.deepcopy(record)
            candidate["assets"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                build.validate_release(candidate, False)

    def test_build_preserves_sources(self):
        for output in (
            build.ROOT, build.SOURCE, build.SOURCE.parent.parent,
            build.SOURCE / "static", build.SOURCE / "static/screenshots/output",
            build.SOURCE / "pages", build.SOURCE / "pages/output",
        ):
            with self.subTest(output=output), self.assertRaises(ValueError):
                build.build(output)

    def test_offline_build_keeps_direct_downloads_and_brand_assets(self):
        with tempfile.TemporaryDirectory(prefix="cor-website-test-") as directory:
            output = Path(directory) / "site"
            with redirect_stdout(io.StringIO()):
                build.build(output)
            for name in build.PAGES:
                text = (output / f"{name}.html").read_text(encoding="utf-8")
                self.assertNotIn("{{", text)
                self.assertIn('lang="zh-CN"', text)
                self.assertIn('class="repository-link"', text)
            download = (output / "download.html").read_text(encoding="utf-8")
            self.assertEqual(download.count(" data-download "), 9)
            self.assertEqual(download.count(" data-checksum "), 9)
            self.assertIn("<noscript>", download)
            self.assertIn('data-interactive hidden', download)
            for name in ("wordmark.svg", "app.png", "LICENSE.txt", "site.css", "site.js"):
                self.assertTrue((output / "assets" / name).is_file(), name)
            headers = (output / "_headers").read_text(encoding="utf-8")
            self.assertIn("connect-src 'self' https://api.github.com", headers)

    def test_failed_refresh_preserves_the_saved_snapshot(self):
        with tempfile.TemporaryDirectory(prefix="cor-website-refresh-") as directory:
            snapshot = Path(directory) / "releases.json"
            snapshot.write_text("existing snapshot", encoding="utf-8")
            with patch.object(build, "SOURCE", Path(directory)):
                with patch.object(build, "request", return_value={"draft": True}):
                    with self.assertRaises(ValueError):
                        build.refresh()
            self.assertEqual(snapshot.read_text(encoding="utf-8"), "existing snapshot")


if __name__ == "__main__":
    unittest.main()
