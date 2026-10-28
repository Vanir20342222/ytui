#!/usr/bin/env bash
# Run only on an explicitly selected disposable emulator/test installation.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
evidence="$root/target/android-device-evidence"
test_package=dev.ytui.android.preview
mkdir -p "$evidence"
# Disposable emulator only. A first-boot Pixel Launcher ANR can cover the app
# and steal IME focus; the launcher is not used by these activity tests.
adb shell am force-stop com.google.android.apps.nexuslauncher || true
adb shell input keyevent KEYCODE_WAKEUP
adb shell wm dismiss-keyguard
adb shell settings put secure show_ime_with_hard_keyboard 1
adb logcat -v threadtime > "$evidence/live-logcat.txt" 2>&1 &
capture_pid=$!
collect_evidence() {
    kill "$capture_pid" 2>/dev/null || true
    wait "$capture_pid" 2>/dev/null || true
    adb logcat -d -b crash -b main -b system > "$evidence/logcat.txt" 2>&1 || true
    adb shell dumpsys activity exit-info "$test_package" > "$evidence/exit-info.txt" 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/video-player.png "$evidence/video-player.png" > /dev/null 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/video-error.png "$evidence/video-error.png" > /dev/null 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/video-texture.png "$evidence/video-texture.png" > /dev/null 2>&1 || true
    mkdir -p "$evidence/native-ui"
    for screenshot in native-search native-account native-tablet native-catalog native-music native-wide-catalog native-player native-player-{vinyl,vu,orbit} native-settings native-card-menu native-video-fullscreen native-theme-{0..12}; do
        adb pull "/sdcard/Android/data/$test_package/files/$screenshot.png" "$evidence/native-ui/$screenshot.png" > /dev/null 2>&1 || true
    done
}
trap collect_evidence EXIT
arguments=(-Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true)
if [[ -n "${YTUI_ANDROID_PUBLIC_URL:-}" ]]; then
    arguments+=("-Pandroid.testInstrumentationRunnerArguments.youtubeUrl=$YTUI_ANDROID_PUBLIC_URL")
fi
apps/android/gradlew -p apps/android --no-daemon "${arguments[@]}" :app:connectedDebugAndroidTest
adb logcat -d -s RustPanic:E RustStdoutStderr:I > "$evidence/rust-runtime.txt"
if rg -q 'RustPanic|panicked at' "$evidence/rust-runtime.txt"; then
    echo 'Rust panic detected despite passing instrumentation; inspect device evidence.' >&2
    exit 1
fi
if rg -Fq "ANR in $test_package" "$evidence/live-logcat.txt"; then
    echo 'Application ANR detected despite passing instrumentation; inspect device evidence.' >&2
    exit 1
fi
