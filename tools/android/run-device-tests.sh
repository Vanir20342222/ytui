#!/usr/bin/env bash
# Run only on an explicitly selected disposable emulator/test installation.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
evidence="$root/target/android-device-evidence"
test_package=dev.ytui.android.preview
mkdir -p "$evidence"
# Google SDK Setup can raise its own first-boot ANR before instrumentation and
# obscure a correctly rendered app. It is not an app dependency. Disable only
# on an emulator, retaining all app/system-dialog assertions during the suite.
if [[ "$(adb shell getprop ro.kernel.qemu | tr -d '\r')" == 1 ]]; then
    adb shell pm disable-user --user 0 com.google.android.googlesdksetup > /dev/null 2>&1 || true
fi
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
    adb pull /sdcard/Android/data/$test_package/files/video-surface.png "$evidence/video-surface.png" > /dev/null 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/startup-timings.txt "$evidence/startup-timings.txt" > /dev/null 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/startup-hls-timings.txt "$evidence/startup-hls-timings.txt" > /dev/null 2>&1 || true
    adb pull /sdcard/Android/data/$test_package/files/startup-indexed-timings.txt "$evidence/startup-indexed-timings.txt" > /dev/null 2>&1 || true
    for name in public-startup-timings startup-subsecond-timings startup-indexed-prefetched-timings; do
        adb pull "/sdcard/Android/data/$test_package/files/$name.txt" "$evidence/$name.txt" > /dev/null 2>&1 || true
    done
    for state in cold prepared missing-frame settings hls hls-prepared indexed indexed-prefetched indexed-prefetched-failed subsecond subsecond-cold subsecond-failed compositor-clear compositor-covered compositor-restored; do
        adb pull /sdcard/Android/data/$test_package/files/startup-$state.png "$evidence/startup-$state.png" > /dev/null 2>&1 || true
    done
    mkdir -p "$evidence/native-ui"
    for motion in restyle-fidelity-motion.mp4 restyle-fidelity-motion.json; do
        adb pull "/sdcard/Android/data/$test_package/files/$motion" "$evidence/$motion" > /dev/null 2>&1 || true
    done
    for screenshot in native-search native-account native-tablet native-catalog native-catalog-error native-music native-wide-catalog native-player native-player-{vinyl,vu,orbit} native-settings native-card-menu native-video-fullscreen native-video-return native-video-controls native-theme-{0..12}; do
        adb pull "/sdcard/Android/data/$test_package/files/$screenshot.png" "$evidence/native-ui/$screenshot.png" > /dev/null 2>&1 || true
    done
    while IFS= read -r screenshot; do
        screenshot="${screenshot//$'\r'/}"
        [[ "$screenshot" =~ ^restyle-[A-Za-z0-9._-]+\.png$ ]] || continue
        adb pull "/sdcard/Android/data/$test_package/files/$screenshot" "$evidence/native-ui/$screenshot" > /dev/null 2>&1 || true
    done < <(adb shell "ls /sdcard/Android/data/$test_package/files/restyle-*.png" 2>/dev/null | sed 's|.*/||')
    adb pull "/sdcard/Android/data/$test_package/files/native-video-controls.xml" "$evidence/native-ui/native-video-controls.xml" > /dev/null 2>&1 || true
    adb pull "/sdcard/Android/data/$test_package/files/watch-collapse-missing.xml" "$evidence/native-ui/watch-collapse-missing.xml" > /dev/null 2>&1 || true
}
trap collect_evidence EXIT
public_quality="${YTUI_ANDROID_PUBLIC_QUALITY:-best}"
public_target="${YTUI_ANDROID_PUBLIC_TARGET_MS:-1000}"
if [[ -n "${YTUI_ANDROID_PUBLIC_URL:-}" ]]; then
    case "$public_quality" in best|1080|720) ;; *) echo 'Invalid public quality.' >&2; exit 1;; esac
    case "$public_target" in 1000|2000) ;; *) echo 'Invalid public target.' >&2; exit 1;; esac
fi
if [[ -n "${YTUI_ANDROID_TEST_APKS:-}" ]]; then
    # CI shards: install the exact APKs built (and uploaded) by the apk job
    # instead of rebuilding them, then run one AndroidJUnitRunner shard.
    num_shards="${YTUI_ANDROID_NUM_SHARDS:-1}"
    shard_index="${YTUI_ANDROID_SHARD_INDEX:-0}"
    [[ "$num_shards" =~ ^[1-9][0-9]?$ && "$shard_index" =~ ^[0-9]+$ ]] && (( shard_index < num_shards )) ||
        { echo 'Invalid YTUI_ANDROID_NUM_SHARDS/YTUI_ANDROID_SHARD_INDEX.' >&2; exit 1; }
    # upload-artifact keeps paths relative to the files' common ancestor
    # (outputs/apk/), so the APKs arrive as debug/… and androidTest/debug/….
    app_apk="$(find "$YTUI_ANDROID_TEST_APKS" -type f -name app-debug.apk | head -n 1)"
    test_apk="$(find "$YTUI_ANDROID_TEST_APKS" -type f -name app-debug-androidTest.apk | head -n 1)"
    [[ -n "$app_apk" && -n "$test_apk" ]] ||
        { echo "Test APKs not found under $YTUI_ANDROID_TEST_APKS:" >&2; find "$YTUI_ANDROID_TEST_APKS" -type f >&2; exit 1; }
    adb install -r -t "$app_apk"
    adb install -r -t "$test_apk"
    runner="$(adb shell pm list instrumentation | tr -d '\r' | sed -n "s/^instrumentation:\([^ ]*\) (target=$test_package)\$/\1/p" | head -n 1)"
    [[ -n "$runner" ]] || { echo "No instrumentation installed for $test_package." >&2; exit 1; }
    instrument=(adb shell am instrument -w -r -e numShards "$num_shards" -e shardIndex "$shard_index")
    if [[ -n "${YTUI_ANDROID_PUBLIC_URL:-}" ]]; then
        # adb joins arguments into one device shell command; single-quote the URL.
        [[ "$YTUI_ANDROID_PUBLIC_URL" != *"'"* ]] || { echo 'Public URL must not contain a single quote.' >&2; exit 1; }
        instrument+=(-e youtubeUrl "'$YTUI_ANDROID_PUBLIC_URL'")
        instrument+=(-e youtubeQuality "$public_quality" -e youtubeTargetMs "$public_target")
    fi
    "${instrument[@]}" "$runner" | tee "$evidence/instrumentation-shard-$shard_index.txt"
    # grep, not rg: emulator shard runners do not ship ripgrep.
    # Raw output ends with "OK (N tests)" only when every test in the shard passed;
    # failures print "FAILURES!!!" and crashes print neither.
    if grep -Eq 'FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed' "$evidence/instrumentation-shard-$shard_index.txt" ||
        ! grep -Eq '^OK \([0-9]+ tests?\)' "$evidence/instrumentation-shard-$shard_index.txt"; then
        echo "Instrumentation shard $shard_index/$num_shards failed; inspect device evidence." >&2
        exit 1
    fi
else
    arguments=(-Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true)
    if [[ -n "${YTUI_ANDROID_PUBLIC_URL:-}" ]]; then
        arguments+=("-Pandroid.testInstrumentationRunnerArguments.youtubeUrl=$YTUI_ANDROID_PUBLIC_URL")
        arguments+=("-Pandroid.testInstrumentationRunnerArguments.youtubeQuality=$public_quality")
        arguments+=("-Pandroid.testInstrumentationRunnerArguments.youtubeTargetMs=$public_target")
    fi
    apps/android/gradlew -p apps/android --no-daemon "${arguments[@]}" :app:connectedDebugAndroidTest
fi
adb logcat -d -s RustPanic:E RustStdoutStderr:I > "$evidence/rust-runtime.txt"
if grep -Eq 'RustPanic|panicked at' "$evidence/rust-runtime.txt"; then
    echo 'Rust panic detected despite passing instrumentation; inspect device evidence.' >&2
    exit 1
fi
if grep -Fq "ANR in $test_package" "$evidence/live-logcat.txt"; then
    echo 'Application ANR detected despite passing instrumentation; inspect device evidence.' >&2
    exit 1
fi
