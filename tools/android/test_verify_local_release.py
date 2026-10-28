"""Synthetic publication checks; no signing key, account or external server used."""

import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock
import zipfile

spec = importlib.util.spec_from_file_location(
    "verify_release", Path(__file__).with_name("verify-local-release.py")
)
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


class PublicationBoundary(unittest.TestCase):
    def test_transfer_rejects_other_hosts_credentials_and_protocols(self):
        for url in [
            "http://a.oaiusercontent.com/a", "https://example.com/a",
            "https://a.oaiusercontent.com.evil.test/a", "https://127.0.0.1/a",
            "https://u:p@a.oaiusercontent.com/a", "https://a.oaiusercontent.com:8443/a",
            "https://a.oaiusercontent.com/a#fragment",
        ]:
            with self.subTest(url=url), self.assertRaises(ValueError):
                verify.transfer_url(url)
        self.assertEqual(
            verify.transfer_url("https://a.oaiusercontent.com/a?synthetic=token"),
            "https://a.oaiusercontent.com/a?synthetic=token",
        )

    def run_fixture(self, *, wrong_digest=False, wrong_source=False,
                    wrong_certificate=False, debuggable=False):
        source = "1" * 40
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            archive.writestr("classes.dex", ("2" * 40 if wrong_source else source).encode())
        payload = buffer.getvalue()
        env = {
            "YTUI_VERSION_NAME": "1.0-preview", "YTUI_VERSION_CODE": "4",
            "YTUI_APK_SHA256": "0" * 64 if wrong_digest else hashlib.sha256(payload).hexdigest(),
            "GITHUB_SHA": source, "YTUI_APK_URL": "https://a.oaiusercontent.com/fixture",
            "ANDROID_HOME": "/synthetic-sdk",
        }
        response = io.BytesIO(payload)
        response.status = 200
        opener = Mock()
        opener.open.return_value = response
        cert = "0" * 64 if wrong_certificate else verify.STABLE_CERT
        signature = f"Signer #1 certificate SHA-256 digest: {cert}\n"
        badging = "package: name='dev.ytui.android.stable' versionCode='4' versionName='1.0-preview' platformBuildVersionName='15'\n"
        if debuggable:
            badging += "application-debuggable\n"
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as folder:
            try:
                os.chdir(folder)
                with patch.dict(os.environ, env), patch.object(
                    verify.urllib.request, "build_opener", return_value=opener
                ), patch.object(verify.subprocess, "check_output", side_effect=[signature, badging]):
                    verify.main()
                self.assertTrue(Path("publish/notes.md").is_file())
                self.assertIn(source, Path("publish/APK-inspection.txt").read_text())
            finally:
                os.chdir(previous)

    def test_valid_transfer_is_traceable(self):
        self.run_fixture()

    def test_tampering_other_sources_keys_and_debug_packages_are_rejected(self):
        for option in ["wrong_digest", "wrong_source", "wrong_certificate", "debuggable"]:
            with self.subTest(option=option), self.assertRaises(ValueError):
                self.run_fixture(**{option: True})


if __name__ == "__main__":
    unittest.main()
