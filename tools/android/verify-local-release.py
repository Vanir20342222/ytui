#!/usr/bin/env python3
"""Verify an APK transfer before publishing; this program never handles signing keys."""

import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile

STABLE_CERT = "b3e9f1a70dd6597884d1dbba09eafad3d4d73ea62c4074a9b8cec1ac1e034848"
LIMIT = 100 * 1024 * 1024


def transfer_url(value):
    url = urllib.parse.urlsplit(value)
    if (url.scheme != "https" or not url.hostname or
        not url.hostname.endswith(".oaiusercontent.com") or url.port not in (None, 443) or
        url.username or url.password or url.fragment):
        raise ValueError("Use a temporary HTTPS file-service APK URL")
    return value


class SecureRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, newurl):
        return super().redirect_request(
            request, response, code, message, headers, transfer_url(newurl)
        )


def main():
    version = os.environ["YTUI_VERSION_NAME"]
    code = os.environ["YTUI_VERSION_CODE"]
    digest = os.environ["YTUI_APK_SHA256"]
    revision = os.environ["GITHUB_SHA"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+_-]{0,63}", version):
        raise ValueError("Invalid version")
    if not re.fullmatch(r"[1-9][0-9]{0,9}", code) or int(code) > 2100000000:
        raise ValueError("Invalid version code")
    if not re.fullmatch(r"[0-9a-f]{64}", digest) or not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("Invalid checksum or source revision")
    url = transfer_url(os.environ["YTUI_APK_URL"])
    output = Path("publish")
    output.mkdir(exist_ok=True)
    apk = output / f"ytui-{version}.apk"
    opener = urllib.request.build_opener(SecureRedirect())
    total = 0
    checksum = hashlib.sha256()
    with opener.open(url, timeout=90) as response, apk.open("xb") as target:
        if response.status != 200:
            raise ValueError("APK transfer failed")
        while block := response.read(128 * 1024):
            total += len(block)
            if total > LIMIT:
                raise ValueError("APK exceeds transfer limit")
            checksum.update(block)
            target.write(block)
    if checksum.hexdigest() != digest:
        raise ValueError("APK checksum mismatch")
    with zipfile.ZipFile(apk) as archive:
        if sum(item.file_size for item in archive.infolist()) > 500 * 1024 * 1024:
            raise ValueError("APK exceeds decompressed limit")
        if not any(revision.encode() in archive.read(name) for name in archive.namelist()
                   if re.fullmatch(r"classes[0-9]*\.dex", name)):
            raise ValueError("APK source marker does not match checked main")
    tools = Path(os.environ["ANDROID_HOME"]) / "build-tools/35.0.0"
    signature = subprocess.check_output(
        [str(tools / "apksigner"), "verify", "--print-certs", str(apk)], text=True
    )
    certs = re.findall(r"^Signer #\d+ certificate SHA-256 digest: (\w+)$", signature, re.M)
    if certs != [STABLE_CERT]:
        raise ValueError("APK is not signed by the stable identity")
    badging = subprocess.check_output(
        [str(tools / "aapt2"), "dump", "badging", str(apk)], text=True
    )
    expected = f"package: name='dev.ytui.android.stable' versionCode='{code}' versionName='{version}'"
    if not badging.startswith(expected + " ") or re.search(r"^application-debuggable", badging, re.M):
        raise ValueError("APK package/version/release mode mismatch")
    (output / "APK-inspection.txt").write_text(f"Source revision: {revision}\n" + signature)
    (output / "SHA256SUMS.txt").write_text(f"{digest}  {apk.name}\n")
    (output / "notes.md").write_text(
        f"Android development preview **{version}**, code **{code}**. Android 8+, ARM64/x86-64.\n\n"
        "Download the APK below and open it from Files. Installs as **ytui** "
        "(`dev.ytui.android.stable`) alongside older Preview apps. No uninstall is needed. "
        "Account/settings/library are separate; sign in and choose folders once. "
        "Subsequent stable builds use this same signing identity.\n\n"
        f"Source `{revision}` passed Android, Rust and Secret scan workflows. "
        "The APK is signed privately, checksum/certificate/ABI/alignment inspected, "
        "and published without rebuilding or re-signing.\n\n"
        "Real YouTube latency, audible output, Galaxy Tab S9 / Nothing Phone (3a), "
        "OEM background behavior and physical in-place update acceptance remain "
        "separate from offline emulator qualification.\n\n"
        f"[Validation](https://github.com/Vanir20342222/ytui/blob/{revision}/docs/android/VALIDATION.md)"
        f" · [Changes](https://github.com/Vanir20342222/ytui/blob/{revision}/docs/android/PLAYER_LIBRARY_PLAN.md)\n"
    )


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Transfer failures must not print the temporary signed URL or query token.
        sys.exit("APK verification failed; no release published. Check checksum, identity, version and source.")
