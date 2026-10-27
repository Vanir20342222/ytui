#!/usr/bin/env bash
# Run only on an explicitly selected disposable emulator/test installation.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
evidence="$root/target/android-device-evidence"
mkdir -p "$evidence"
adb logcat -v threadtime > "$evidence/live-logcat.txt" 2>&1 &
capture_pid=$!
collect_evidence() {
    kill "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
    adb logcat -d -b crash -b main -b system > "$evidence/logcat.txt" 2>&1 || true
    adb shell dumpsys activity exit-info dev.ytui.android > "$evidence/exit-info.txt" 2>&1 || true
    adb pull /sdcard/Android/data/dev.ytui.android/files/video-player.png "$evidence/video-player.png" > /dev/null 2>&1 || true
}
trap collect_evidence EXIT
apps/android/gradlew -p apps/android --no-daemon :app:connectedDebugAndroidTest
