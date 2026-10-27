# Android validation record — 2026-10-27 (M9)

Android is a development preview. No physical Nothing Phone (3a) or Galaxy Tab S9
has been connected. No personal cookies, browser database, VPN or desktop
appearance settings were used or changed by these tests. Fixture decoding,
public-provider checks and hardware acceptance are recorded separately.

## Passed checks

- Final local Rust workspace suite: **293 passed, 0 failed, 31 explicitly ignored**.
  Formatting and strict all-target workspace Clippy passed. ARM64 Android-specific
  JNI/GLES code also passed strict cross-target Clippy.
- Mobile checks cover all 13 theme entries at 360×800, 412×915 and 1280×800 logical
  layouts; four tabs and overlays; touch navigation; Back overlay order; three
  music styles, pause/reduced motion; settings/download draft cancellation;
  malformed Unicode palettes; stale playback/account replies; feed cooldown;
  bounded requests/images; scoped synthetic sessions; durable saved/queue/resume;
  failed-write rollback; native format selection and SponsorBlock-off privacy.
  These are headless Rust UI checks, not hardware performance measurements.
- Desktop saved-player Node challenge vectors: **2 passed**. Hosted
  [Rust run 40543256434](https://github.com/Vanir20342222/ytui/actions/runs/40543256434)
  passed desktop/shared checks, real FFmpeg/mpv/local-playlist checks and release
  build after the monorepo move. Desktop launchers/state paths are unchanged.
- Complete local Android build: ARM64 and x86-64 native libraries, dependency
  notices, Gradle wrapper, debug/instrumentation APK assembly, **3 Java unit tests**
  and Android Lint with **0 errors**. APK inspection passed signing, ZIP and
  **16 KiB ELF** alignment, both ABIs, JNI/GameActivity symbols and bundled notices.
- Accelerated API 35
  [Android run 40543256859](https://github.com/Vanir20342222/ytui/actions/runs/40543256859)
  passed **5 device tests, 0 failures**. The separate opt-in public YouTube case
  was explicitly skipped in this default, account-free run. Passed device checks:
  - Generated MP4 actually decodes into the Rust GLES surface and produces PCM
    levels; pause and volume persistence across stream changes work.
  - Short muted preview restores the previous media/position/preferences on EOS.
  - Actual loopback HTTP download plays from private offline cache after its
    server is closed.
  - Closing/reopening the Rust UI in the same process recreates the UI and leaves
    Android's main loop responsive.
  - JNI contracts/direct-buffer bounds and empty audio EOS input are checked.
  - Android's isolated JavaScript runtime has no browser/network/cookie APIs;
    original saved YouTube n/signature vectors return the exact expected maps.

The run's device-evidence artifact contains JUnit results, continuous logcat,
exit information and an actual 1080×2400 player screenshot. Visual inspection
confirmed system-bar separation and visible sliders/clock labels. It is generated
local test media, not evidence of public YouTube playback or audible output.

## Development APK

The earlier five-test checkpoint is source `aad9486` (PR merge revision `0d98630`);
log review subsequently exposed a caught shutdown panic, described below. It is
retained as historical evidence and must be replaced by a clean-shutdown build:
[APK and inspection reports](https://github.com/Vanir20342222/ytui/actions/runs/40543256859/artifacts/12750509647).
The ZIP contains `outputs/apk/debug/app-debug.apk` and `SHA256-and-checks.txt`.
The APK's SHA256 is:

```text
e3b472dc6f3c1caf18b7b5baa864397191214482079d6a2108f423cf42c62a5d
```

Artifacts expire after 30 days. This debug APK is not a stable signed release;
different CI signing identities cannot update each other in place. Read
[installation instructions](README.md#install-an-apk) before replacing an install.

## Public YouTube acceptance

The explicit, cookie-free
[public run 40543378151](https://github.com/Vanir20342222/ytui/actions/runs/40543378151)
failed before network access because the test sent `kind: Video` rather than
the Rust API's lowercase `video`. Both media test fixtures now use the actual
contract. The rerun tests native extraction and at least three seconds of actual
playback of `https://www.youtube.com/watch?v=jNQXAC9IVRw`; its result is recorded
separately after completion. No public Android playback success is claimed from
fixture or saved-player checks. The workflow's `public_url` input makes this
reproducible without making ordinary tests depend on YouTube availability.

## Reproduced runtime bugs fixed during acceptance

Earlier accelerated runs exposed real failures; their first APKs are not accepted:

- `40519913127`: playback foreground-service deadline. Explicitly register the
  MediaSession and post a preparing notification before decoder readiness.
- `40528348716`: empty EOS audio input copied the shared empty ByteBuffer into
  itself. Return before copying empty input; device regression now passes.
- The same run found Activity teardown blocking in GameActivity termination.
  Documented Android-only winit Destroy/loop-recreation patch fixes shutdown;
  close/reopen/main-loop responsiveness now pass in the same app process.
- `40532272312`: offline foreground service used string-resource ID zero. Use a
  real channel-name resource; actual download/offline decoding now pass.
- `40536947562`: four of five passed; saved-player test incorrectly expected
  response `type` to equal `n`/`sig` rather than EJS's `result`. Corrected the
  envelope assertion while retaining exact key-count/solution-map validation;
  both challenge vectors passed in the subsequent green run.
- Reviewing `40543256859` logcat exposed caught Rust shutdown panics despite
  passing assertions: eframe's exit path unwrapped a window already removed on
  Android Suspended, skipping application cleanup. A documented eframe 0.33.3
  patch saves with an optional window and restores a surfaceless EGL context for
  deletion where supported. The test now requires the Rust exit hook to complete
  twice; the device runner also fails on Rust panic logs. Cross-target Clippy
  passes; runtime qualification requires the new run.

The cloud host lacks `/dev/kvm`; its software emulator was unreliable and an
earlier instrumentation attempt crashed. Those attempts are not counted as
passes. The accelerated hosted emulator supplied the successful runtime evidence.

## Acceptance still required

On both requested physical devices: install/update signing identity, soft keyboard,
rotation, tablet split layout, gesture/system insets, accessibility, every
settings/modal screen, all themes/player styles, real video and Music playback,
codec/quality/track/caption selection, HTTP/HTTPS stations, SponsorBlock timing,
account expiry/disconnect, multi-page feeds/playlists, paired offline media,
network loss/VPN handoff/reconnect/rate-limit cooldown, process death, screen lock,
audio focus/headset/Bluetooth, notifications, background limits, measured
memory/CPU/frame latency and human audible confirmation. Advanced capability gaps
remain explicit in [PARITY.md](PARITY.md). M9 remains open.
