#!/usr/bin/env python3
"""Prepare an inspected CI Preview APK for publication, without changing its signature."""

import hashlib
import os
from pathlib import Path
import re
import shutil
import zipfile


def prepare(tested, output, revision, run, tag):
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Invalid source revision")
    if not re.fullmatch(r"android-preview-[A-Za-z0-9][A-Za-z0-9.+_-]{0,63}", tag):
        raise ValueError("Invalid preview tag")
    if not re.fullmatch(r"[1-9][0-9]*", run):
        raise ValueError("Invalid Android workflow run")
    candidates = list(tested.rglob("app-debug.apk"))
    if len(candidates) != 1:
        raise ValueError("Expected exactly one tested Preview APK")
    apk = candidates[0]
    report = apk.with_name("SHA256-and-checks.txt").read_text()
    digest = hashlib.sha256(apk.read_bytes()).hexdigest()
    hashes = re.findall(r"^([0-9a-f]{64})\s+\S*app-debug\.apk$", report, re.M)
    if hashes != [digest] or "Verified signatures, ZIP alignment, both Rust ABIs" not in report:
        raise ValueError("APK does not match its CI inspection")
    with zipfile.ZipFile(apk) as archive:
        for abi in ("arm64-v8a", "x86_64"):
            archive.getinfo(f"lib/{abi}/libytui_mobile.so")
        archive.getinfo("assets/notices/inventory.json")
        archive.getinfo("assets/ytui-LICENSE")
        if not any(revision.encode() in archive.read(name) for name in archive.namelist()
                   if re.fullmatch(r"classes[0-9]*\.dex", name)):
            raise ValueError("APK source differs from the checked main revision")
    output.mkdir()
    name = f"ytui-{tag.removeprefix('android-')}.apk"
    shutil.copyfile(apk, output / name)
    (output / "SHA256SUMS.txt").write_text(f"{digest}  {name}\n")
    (output / "APK-inspection.txt").write_text(f"Source revision: {revision}\n" + report)
    (output / "notes.md").write_text(
        "Android **development Preview** combining the startup optimizations, strict playback "
        "evidence, SurfaceView rendering, onboarding, recommendations and desktop integration.\n\n"
        "Download the `.apk` below and open it from Files. Android 8+, ARM64 and x86-64. "
        "Installs as **ytui Preview** (`dev.ytui.android.preview`) alongside the stable app. "
        "It has separate settings, account and library data. This is a CI debug-signed build, "
        "not an update for the permanent stable signing identity. Earlier Preview builds may "
        "have a different debug key; preserve existing installations and their private data.\n\n"
        f"Source `{revision}` passed the latest exact-main Android, Rust and Secret scan runs. "
        f"This is the exact APK from [Android run {run}]"
        f"(https://github.com/Vanir20342222/ytui/actions/runs/{run}), including its emulator checks. "
        "It was not rebuilt or re-signed. Check `SHA256SUMS.txt` and `APK-inspection.txt`.\n\n"
        "Physical Galaxy Tab S9 / Nothing Phone (3a) acceptance, live YouTube startup latency, "
        "audible output, account/network behavior and permanent-key stable updates remain "
        "separate from offline CI qualification.\n\n"
        f"[Validation](https://github.com/Vanir20342222/ytui/blob/{revision}/docs/android/VALIDATION.md)\n"
    )


if __name__ == "__main__":
    prepare(Path("tested"), Path("publish"), os.environ["YTUI_PREVIEW_SOURCE"],
            os.environ["YTUI_PREVIEW_RUN"], os.environ["GITHUB_REF_NAME"])
