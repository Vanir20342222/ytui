#!/usr/bin/env python3
"""Build with the privately recovered owner identity; never output credentials."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys

STABLE_CERT = "b3e9f1a70dd6597884d1dbba09eafad3d4d73ea62c4074a9b8cec1ac1e034848"


def private_path(path, directory=False):
    if path.is_symlink():
        raise ValueError("Signing files cannot be symbolic links")
    mode = path.stat().st_mode
    if stat.S_IMODE(mode) & 0o077:
        raise ValueError("Signing files and their directory must be owner-only")
    if not (stat.S_ISDIR(mode) if directory else stat.S_ISREG(mode)):
        raise ValueError("Invalid signing file or directory")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("identity", type=Path, help="Private recovered signing directory")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    identity = args.identity.resolve()
    if identity.is_relative_to(root):
        raise ValueError("Keep the signing identity outside the repository")
    private_path(identity, directory=True)
    keystore = identity / "ytui-stable.jks"
    credentials = identity / "credentials.json"
    private_path(keystore)
    private_path(credentials)
    values = json.loads(credentials.read_text())
    fields = {"alias", "store_password", "key_password"}
    if not all(isinstance(values.get(key), str) and values[key] for key in fields):
        raise ValueError("Incomplete recovered signing credentials")
    if subprocess.check_output(["git", "status", "--porcelain"], cwd=root).strip():
        raise ValueError("Commit or stash changes before signing a traceable release")
    env = os.environ.copy()
    env.update(
        YTUI_ANDROID_VARIANT="release",
        YTUI_ANDROID_KEYSTORE=str(keystore),
        YTUI_ANDROID_KEY_ALIAS=values["alias"],
        YTUI_ANDROID_STORE_PASSWORD=values["store_password"],
        YTUI_ANDROID_KEY_PASSWORD=values["key_password"],
    )
    certificate = subprocess.check_output(
        ["keytool", "-exportcert", "-alias", values["alias"], "-keystore", str(keystore),
         "-storepass:env", "YTUI_ANDROID_STORE_PASSWORD"],
        env=env, stderr=subprocess.PIPE,
    )
    if hashlib.sha256(certificate).hexdigest() != STABLE_CERT:
        raise ValueError("The identity does not match the existing stable certificate")
    subprocess.run([str(root / "tools/android/build.sh")], cwd=root, env=env, check=True)
    subprocess.run(
        [str(root / "tools/android/inspect-apk.sh"),
         str(root / "apps/android/app/build/outputs/apk/release/app-release.apk")],
        cwd=root, check=True,
    )


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, json.JSONDecodeError):
        # Do not stringify exceptions: parsers/OS errors may include private input.
        sys.exit("Could not use the private identity. Check paths, owner-only permissions and credentials.")
    except subprocess.CalledProcessError:
        sys.exit("Release build or inspection failed; no APK should be published.")
