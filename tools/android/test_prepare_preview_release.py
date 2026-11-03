import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location(
    "prepare_preview_release", Path(__file__).with_name("prepare-preview-release.py")
)
preview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)


class PreviewPublicationTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.tested = self.root / "tested"
        self.tested.mkdir()
        self.apk = self.tested / "app-debug.apk"
        self.revision = "a" * 40
        self.fixture()

    def fixture(self, source=None, abis=("arm64-v8a", "x86_64")):
        with zipfile.ZipFile(self.apk, "w") as archive:
            archive.writestr("classes.dex", source or self.revision)
            for abi in abis:
                archive.writestr(f"lib/{abi}/libytui_mobile.so", b"fixture")
            archive.writestr("assets/notices/inventory.json", "{}")
            archive.writestr("assets/ytui-LICENSE", "MIT")
        digest = hashlib.sha256(self.apk.read_bytes()).hexdigest()
        self.apk.with_name("SHA256-and-checks.txt").write_text(
            "Verified signatures, ZIP alignment, both Rust ABIs, 16 KiB ELF segments and notices.\n"
            f"{digest}  /build/app-debug.apk\n"
        )

    def prepare(self):
        preview.prepare(self.tested, self.root / "publish", self.revision, "123",
                        "android-preview-0.2.0-alpha.12-mobile.11")

    def test_published_apk_is_byte_identical_to_tested_artifact(self):
        self.prepare()
        published = list((self.root / "publish").glob("*.apk"))
        self.assertEqual(published[0].read_bytes(), self.apk.read_bytes())

    def test_altered_apk_is_rejected(self):
        with self.apk.open("ab") as stream:
            stream.write(b"changed")
        with self.assertRaisesRegex(ValueError, "CI inspection"):
            self.prepare()
        self.assertFalse((self.root / "publish").exists())

    def test_wrong_source_is_rejected_even_with_valid_checksum(self):
        self.fixture(source="b" * 40)
        with self.assertRaisesRegex(ValueError, "checked main"):
            self.prepare()

    def test_missing_abi_is_rejected(self):
        self.fixture(abis=("arm64-v8a",))
        with self.assertRaises(KeyError):
            self.prepare()

    def test_ambiguous_artifacts_are_rejected(self):
        other = self.tested / "other"
        other.mkdir()
        (other / self.apk.name).write_bytes(self.apk.read_bytes())
        with self.assertRaisesRegex(ValueError, "exactly one"):
            self.prepare()
