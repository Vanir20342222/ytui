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
  notices, Gradle wrapper, debug/instrumentation APK assembly, **5 Java unit tests**
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
retained only as historical evidence. The clean-shutdown follow-up at `fa48079`
passed [Android run 40547062980](https://github.com/Vanir20342222/ytui/actions/runs/40547062980)
and Rust run `40547062981`: **5 device tests passed**, both exit hooks completed,
and the panic-log gate recorded **zero Rust panics**. Its newer APK supersedes
the first checkpoint below; the export follow-up needs its own runtime result.
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
contract. [Corrected run 40546258421](https://github.com/Vanir20342222/ytui/actions/runs/40546258421)
reached the native extractor and failed with **“All player responses are invalid.
Your IP is likely being blocked by Youtube”** for
`https://www.youtube.com/watch?v=jNQXAC9IVRw`. Its other five tests passed.
Public Android playback remains unqualified on this hosted runner; no fixture or
saved-player success is substituted for it. The workflow's `public_url` input
makes this check reproducible on another network without an account.

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
  passes; run `40547062980` subsequently passed both cleanup assertions and the
  zero-panic gate.

## Offline export follow-up

Rust UI now offers Export file on completed downloads. The Android storage
adapter copies original containers or muxes compatible separate tracks from
cache, writes via the document picker, and runs a cancellable foreground worker.
Queued song/station classification now uses the lowercase Rust JSON contract;
the previous uppercase comparison incorrectly requested video for Music queues.
Five Java unit tests/lint, instrumentation assembly and focused Rust/Clippy checks
pass locally. The added device test downloads separate generated tracks, closes
both source servers, muxes and actually decodes an exported file, verifies a
foreground audio export is byte-for-byte identical, and checks cancellation.
Its actual hosted result must pass before claiming export runtime acceptance.
The first export run (`40576469530`) exposed AAC priming samples with negative
timestamps: the mux loop treated them as EOS and wrote zero audio samples.
It now identifies EOS by track index, shifts every track by the same time origin
and rejects an output with any empty track. The device regression retains both
track checks and additionally requires decoded PCM from the exported file.

Android's platform MP4 writer does not accept Opus. Normal capped video modes
therefore prefer H.264/AAC, retaining bounded codec fallbacks; unrestricted Best
and audio-only keep their existing best-format behavior. Selector tests include
both AAC and higher-ranked Opus, and verify that missing AAC still resolves.
Unsupported export combinations are rejected before the document picker.

The vendored player fixtures' public client keys are sanitized. Both exact Node
challenge vectors still pass. Secret scanning now checks all fetched history;
see [the provenance/security review](../SECURITY_REVIEW_2026-10-27.md) for the
specific legacy public-fixture baseline and the API permission limit on closing
GitHub's historical alerts.

## Navigation and queue follow-up

Back now restores per-tab pages and scroll positions; older evicted pages reload
automatically, and account changes clear private navigation history. Each tab's
history is bounded to twelve steps and 2 MiB of page data. Late replies cannot
overwrite a restored feed. The virtualized queue has touch reorder buttons,
durable edits and identity checks that prevent a stale remove/reorder action from
changing the wrong entry after the decoder consumes an item. Failed writes leave
the engine and UI queue unchanged; dismissed loading queues stay closed.

The full local workspace checkpoint passed **296 tests, 0 failures, 31 ignored**,
followed by the additional passing queue-dismissal regression. The focused mobile
suite now passes **23 tests** and strict Clippy; the next complete hosted run must
qualify the resulting **297-test** workspace and updated APK.
Safe-area contracts also include left/right display cutouts for phone landscape;
headless geometry checks cover all four edges. Physical rotation acceptance
remains separate.

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
