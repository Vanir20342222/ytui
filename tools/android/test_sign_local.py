"""Offline checks for the private signing boundary, with synthetic inputs only."""

import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("sign-local.py")
spec = importlib.util.spec_from_file_location("sign_local", SCRIPT)
sign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sign)


class SigningBoundary(unittest.TestCase):
    def test_rejects_shared_files_and_symlinks(self):
        with tempfile.TemporaryDirectory() as folder:
            private = Path(folder) / "credentials.json"
            private.write_text("synthetic-only")
            private.chmod(0o600)
            sign.private_path(private)
            private.chmod(0o644)
            with self.assertRaises(ValueError):
                sign.private_path(private)
            private.chmod(0o600)
            link = Path(folder) / "link"
            link.symlink_to(private)
            with self.assertRaises(ValueError):
                sign.private_path(link)
            with self.assertRaises(ValueError):
                sign.private_path(Path(folder))

    def test_invalid_recovery_does_not_echo_credentials(self):
        with tempfile.TemporaryDirectory() as folder:
            for name in ["ytui-stable.jks", "credentials.json"]:
                path = Path(folder) / name
                path.write_text("synthetic-private-value-must-not-appear")
                path.chmod(0o600)
            result = subprocess.run(
                [sys.executable, str(SCRIPT), folder], capture_output=True, text=True
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("synthetic-private-value", result.stdout + result.stderr)
            self.assertNotIn("Traceback", result.stderr)

    def test_refuses_identity_in_repository(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(SCRIPT.parent)],
            capture_output=True, text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
