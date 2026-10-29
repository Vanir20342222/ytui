# Android validation record (M9)

## Player/library preview: signed and qualified (2026-10-29)

Source `cf8046508b96962f1b5ba9efc3529c4427d64b9f` passes [main Android](https://github.com/Vanir20342222/ytui/actions/runs/40837593351), [Rust](https://github.com/Vanir20342222/ytui/actions/runs/40837593137) and Secret scan. **24 runtime cases pass**, one public opt-in case skips; all 13 themes, visible video behind native controls, fullscreen return, native transports, offline folders/playlists and cover recovery pass. Logs contain zero app ANRs/Rust panic markers. The local signed release passes both ABI builds, 14 JVM tests, lint and signature/alignment/notices inspection.

### Current direct APK

[Download 0.2.0-alpha.12-mobile.4](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.4/ytui-0.2.0-alpha.12-mobile.4.apk) · [Public checksum/inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.4). Package `dev.ytui.android.stable`, launcher **ytui**, code **4**, Android 8+, ARM64/x86-64. Installs separately from old Preview apps; subsequent stable updates retain this signing identity. No GitHub signing secrets are required.

SHA256: `4eeaa2145a0b7b08328d940e84c16e1375e481c859f48ac5127e3598301b05c4`. The public download was independently checked. [Full qualification, fixes and open gates](PLAYER_LIBRARY_VALIDATION_2026-10-29.md) · [Actual screenshots](../screenshots/android/README.md). Live YouTube latency, audible output and physical-device/update acceptance remain open. Earlier sections below are historical checkpoints.


Android is a development preview. No physical Nothing Phone (3a) or Galaxy Tab S9
has been connected. No personal cookies, browser database, VPN or desktop
appearance settings were used or changed by these tests. Fixture decoding,
public-provider checks and hardware acceptance are recorded separately.

## Earlier modern media UI checkpoint (2026-10-28): passed

Source **`5f340830c7091aa4546be0e2290560ddbd0bc3ea`**, PR merge revision
**`bb0aa7be25668696f6f41c74c165946799975c75`**.
[Android 40712963078](https://github.com/Vanir20342222/ytui/actions/runs/40712963078),
[Rust 40712962600](https://github.com/Vanir20342222/ytui/actions/runs/40712962600)
and [Secret scan 40712962670](https://github.com/Vanir20342222/ytui/actions/runs/40712962670)
passed. The UI now uses sans-serif typography, artwork grids, phone navigation /
tablet rail, a mini-player and grouped native settings/account/download surfaces.

- **317 offline Rust tests passed**, 0 failed, 31 explicitly ignored; formatting
  and strict hosted Clippy pass. The hosted Rust media regression workflow passes.
- **14 Android JVM tests pass**, including AA accent contrast across 381 colours
  and finite progress with stopped/unknown/invalid telemetry. Debug/release
  compile, unit tests and lint pass locally; hosted debug build/tests/lint pass.
- **20 API 35 emulator cases passed**, 0 failures/errors; the separate public
  YouTube case was explicitly skipped. Continuous logs contain **zero app ANRs
  and zero Rust panic markers**.
- Actual keyboard/Unicode input, all 13 theme backgrounds/foregrounds/persistence,
  account navigation/cancel, native menus/panels, cooldown and URL intent pass.
- New populated UI checks pass thumbnail HTTP/decode/rendered pixels, responsive
  video grids and square Music covers, overflow actions, native settings and
  real Media3 pause/resume/stop through mini-player and expanded transport.
  The tablet setup cold-launches at 2560×1600; a separate case checks live rotation.
- Existing visible RGB video, quality/position/pause, persisted volume, preview
  restoration, cache-only playback, mux/copy/cancel exports alongside playback,
  queue/EOF/history, ICY metadata, exact isolated challenge vectors, lyrics and
  encrypted account/tamper/migration/disconnect checks also pass.

[Phone/tablet/player/settings gallery](../screenshots/android/README.md) contains
unaltered captures from this run; representative catalog, tablet, Music, settings
and player images were visually reviewed. Generated artwork and catalog entries
are test-only; no production provider failure is replaced with demo results.
All theme/keyboard/account captures, JUnit and logs are in the
[device evidence](https://github.com/Vanir20342222/ytui/actions/runs/40712963078/artifacts/12828725236).
Emulator software decoding does not establish physical hardware or audible output.

### Earlier direct APK

[Download ytui Preview](https://github.com/Vanir20342222/ytui/releases/download/android-media-preview-2026-10-28/ytui-preview-0.2.0-alpha.12-mobile.3.apk)
or open the [preview assets/checksum](https://github.com/Vanir20342222/ytui/releases/tag/android-media-preview-2026-10-28).
Version **`0.2.0-alpha.12-mobile.3`**, code **3**, ARM64/x86-64, Android 8/API 26+.
Package **`dev.ytui.android.preview`**, launcher **ytui Preview**. It installs
alongside the old package without uninstalling or changing its private data;
account/settings/library are separate. The normal release identity is unchanged.

The publisher checks successful Android/Rust/Secret scan runs and the exact APK
hash, then republishes the existing tested artifact without rebuilding/re-signing.
Independent inspection verified signing, ZIP / 16 KiB ELF alignment, both native
ABIs, JNI exports, notices, app label, package and version. SHA256:

```text
2e184784b3052b3f15b2a66b0ea7636c8b6f46ed58fc0d1772372e45d7ccaf47
```

This is a development preview. Real Tab S9 / Nothing Phone, live YouTube/account,
audible output, OEM background behaviour and owner stable signing remain open.
Public YouTube playback is not qualified by these offline fixtures. See
[device acceptance](DEVICE_ACCEPTANCE.md), [parity gaps](PARITY.md) and
[signing/update requirements](SIGNING.md). Original artifacts expire; prerelease
assets provide a direct download without a GitHub sign-in or ZIP extraction.

## Modern media UI redesign: runtime validation in progress

The user confirmed installation and explicitly requested replacing terminal
styling with a YouTube/Netflix-inspired mobile design. Source `2c7cca5` adds
artwork grids, native icon navigation, a mini-player and grouped native panels;
see [the design and acceptance requirements](MEDIA_UI_REDESIGN.md).

Local debug/release compilation, lint, APK signing/alignment/ABI inspection,
317 offline Rust tests and 13 Android JVM tests passed. The hosted Rust and
Secret scan workflows also passed for that source. New populated UI tests use
original generated artwork over loopback HTTP; they never substitute for real
provider feeds in the app.

[First redesign Android run 40707245687](https://github.com/Vanir20342222/ytui/actions/runs/40707245687)
planned 21 cases but the process stopped after two: the Music/settings case
passed and the player case crashed with `current must not be NaN`. The deferred
mini-player progress callback read a stopped zero-duration state between layout
and recomposition. The remaining suite did not run; this APK is not qualified.
[Failure evidence](https://github.com/Vanir20342222/ytui/actions/runs/40707245687/artifacts/12825705781)
retains logs, JUnit and screenshots. The fix captures a finite normalized progress
value, guards unknown telemetry and tests stopping playback with the UI attached.
The subsequent [Android run 40710108292](https://github.com/Vanir20342222/ytui/actions/runs/40710108292)
for `9e38ddf` passed **20 cases**, with the public YouTube case explicitly skipped.
The stop/mini-player regression and all prior video, export, queue, ICY, account,
solver and native UI cases passed. Fourteen Android unit tests pass in debug and
release locally; hosted Rust and Secret scan also pass. Rendered warm artwork
pixels are checked before screenshots. The first 2560×1600 screenshot caught an
old popup surface during two emulator size/density overrides despite the grid
geometry checks passing. Tablet screenshot setup now cold-launches the activity
at the target viewport; the separate live-rotation test remains intact.

Development packaging is being separated as `dev.ytui.android.preview` after an
actual certificate comparison proved the installed mobile.2 APK and a new CI
build cannot update each other. Normal release identity stays unchanged. The
new package and cold-start screenshot setup pass in the current checkpoint above.

The earlier metadata-only `fdd047c` Android run 40682419478 failed while installing
the emulator system image (`Error on ZipFile unknown archive`), before any device
tests ran. Bounded system-image download retries are now part of the workflow.

## Previous native UI migration checkpoint: passed emulator checks

Source **`4b2e988`**, PR merge revision **`494627636f39e34b4be5fbb18ee2fa8bd012a616`**.
[Android run 40668747753](https://github.com/Vanir20342222/ytui/actions/runs/40668747753),
[Rust run 40668747755](https://github.com/Vanir20342222/ytui/actions/runs/40668747755)
and [Secret scan 40668747792](https://github.com/Vanir20342222/ytui/actions/runs/40668747792)
passed. This supersedes the failed native UI development attempts below.

- **317 offline Rust tests passed**, 0 failed, 31 explicitly ignored; formatting
  and strict workspace Clippy passed. ARM64 JNI cross-target Clippy passed during
  the native-host migration; both native ABIs rebuilt successfully in this run.
- **12 Android JVM unit tests passed** in both debug and release configurations.
  Kotlin/Java compilation, instrumentation assembly, debug/release lint passed.
- **17 accelerated API 35 emulator checks passed**, 0 failures, 0 errors. The
  separate public YouTube case was explicitly skipped. Continuous logcat has
  **zero Rust panic markers and zero app ANRs**.
- All six native UI checks pass: actual software-keyboard visibility and shell/
  Unicode text input; all 13 shared themes' painted background, foreground and
  persistence; account-error navigation, header editing and explicit sign-in
  cancellation; wide-layout tabs/panels; cooldown prefetch; playlist/Music URL intent.
- Generated video is visibly rendered in the full screenshot, with actual RGB
  pixels. Pause, quality replacement/position, volume across stream changes,
  muted preview restoration and cache-only playback pass. Concurrent export now
  preserves the media session and playback notification; muxed output decodes
  with PCM, audio copies byte-for-byte and cancellation leaves no output file.
- EOF/manual queue/history/controller navigation, ICY metadata/session titles,
  JNI bounds, isolated original n/signature challenge vectors, lyrics and
  Keystore tamper/migration/disconnect checks pass again.

The [device evidence](https://github.com/Vanir20342222/ytui/actions/runs/40668747753/artifacts/12808525396)
contains JUnit results, logs, a visible generated-video screenshot and 16 native
UI screenshots (13 themes, keyboard input, account controls and wide layout).
Representative keyboard, dark/light theme, account, wide-layout and video images
were visually inspected; all 13 palettes also have automated color checks.
Offline debug emulator video uses Android's real
software codec to avoid Goldfish's host-FFmpeg buffer bridge; this does not prove
physical hardware decoding or audible output. The instrumentation re-invocation
used an unknown About build marker; functional source/version matches the APK.
The workflow now carries the source marker across all Gradle steps to prevent
that metadata-only difference in subsequent runs.

### Tested development APK

[Download the APK directly](https://github.com/Vanir20342222/ytui/releases/download/android-native-preview-2026-10-28/ytui-android-0.2.0-alpha.12-mobile.2.apk)
or open the [preview assets and checksum](https://github.com/Vanir20342222/ytui/releases/tag/android-native-preview-2026-10-28).
No GitHub sign-in or ZIP extraction is needed. The preview publisher verifies
the checkpoint's Android/Rust/Secret scan successes and exact SHA256, then
publishes the existing APK without rebuilding or re-signing it.
The [original artifact and reports](https://github.com/Vanir20342222/ytui/actions/runs/40668747753/artifacts/12807657622)
remain available to signed-in GitHub users; extract
`outputs/apk/debug/app-debug.apk` from that ZIP. Version is
**`0.2.0-alpha.12-mobile.2`**, code **2**, with ARM64/x86-64 and Android 8/API 26+.
Local independent inspection verified signing, ZIP and 16 KiB ELF alignment,
both Rust ABIs, required JNI exports and notices. APK SHA256:

```text
2587c533994e69a340730fbe7ba6400cbf80f6216844dfa46ad3bd711fd84791
```

This is an emulator-checked development preview, not a stable signed release.
Artifacts expire after 30 days. Different debug signing identities cannot update
each other in place; follow [installation/update guidance](README.md#install-an-apk)
and preserve wanted app data before replacing an installation.

**Still required:** [Tab S9/phone acceptance](DEVICE_ACCEPTANCE.md), real YouTube/
account/network and audible output checks, hardware/OEM background behavior,
remaining capability gaps and stable owner signing. Public playback is not
qualified by these generated fixtures. At that checkpoint PR #20 was draft; it has since been merged by the owner.
M9 remains a development preview while the physical/live/signing gates are open.

## Physical UI failure and native migration (2026-10-28)

The user installed the earlier development APK on a Galaxy Tab S9 and reported
unusable text fields, theme choices with no visible effect, and protected-feed
errors offering only desktop F2/CLI instructions. This **failed physical UI
acceptance** supersedes earlier impressions from headless renders and backend
checks. The old APK below is a historical backend checkpoint, not a recommended
UI-qualified build.

Production Android presentation now uses Jetpack Compose/ComponentActivity and a
native Media3 PlayerView. The shared Rust engine, private paths and data formats
are preserved. Native text/IME, all-theme painted-pixel/persistence, account-error
navigation/sign-in cancellation and wide-layout panel tests are added. Their
emulator results and a physical Tab S9 re-test must pass before qualification.

Local complete ARM64/x86-64 builds, Kotlin/Java compilation, instrumentation APK
assembly, **9 Java unit tests**, Android lint and APK signing/16 KiB alignment
inspection passed. The full Rust workspace passed **315 tests, 0 failed, 31
explicitly ignored**, formatting and strict host/ARM64 Clippy. Native UI emulator
results will be recorded after the new suite runs. Earlier test counts below describe
their specific Rust/legacy UI/backend checkpoints only.

### First native UI emulator run

[Run 40655604212](https://github.com/Vanir20342222/ytui/actions/runs/40655604212)
at `a0d448f` passed **14 cases**, including all four new native keyboard/theme/
account/wide-layout checks. One generated-video visibility check failed, and the
separate public YouTube case was explicitly skipped. This is not a qualified APK.

Actual shell/IME text entry and Unicode editing worked, every shared theme's
preference persisted and non-system palettes matched painted pixels, the
protected-feed account action opened native sign-in/import controls, Cancel did
not connect an account, and landscape tabs/panels stayed reachable.

The decoder produced frames/PCM, but the screenshot lacked visible RGB video.
The follow-up uses a native TextureView inside Compose's layered player dialog;
the regression now pauses on a decoded frame before screenshot synchronization
can outlast the eight-second fixture, retains screenshot evidence even on
failure, and checks actual colored pixels. Full media/preview/volume/offline
regressions must pass after this change. This does not qualify public YouTube.

Review also found that a zero-delay transient feed error lost its retryability,
and scroll prefetch could restart a failed continuation during its cooldown.
The follow-up retains an explicit retryable flag, bounded backoff, a no-prefetch
error guard and continuation-specific retry while preserving loaded cards. It
adds Rust, Kotlin and native UI regressions. Theme/import updates are serialized;
seek gestures retain their draft position, SponsorBlock marks/replay remain
reachable, and visualizer sampling/animation runs only while visible/playing.

### Second run: graphics/launcher failure and text contrast review

[Run 40658107009](https://github.com/Vanir20342222/ytui/actions/runs/40658107009)
at `743cd00` passed **14 cases**, failed video visibility and keyboard visibility,
and explicitly skipped the public case. The new continuation-cooldown guard
passed. Screenshots revealed a **Pixel Launcher ANR dialog covering the app**;
logcat places the launcher ANR before the test app started. The emulator's
Goldfish host-FFmpeg AVC bridge also reported invalid color-buffer IDs. This run
is not qualified and its development APK is not recommended.

The follow-up uses current SwiftShader mode with cold snapshots disabled, stops
only the unused Pixel Launcher before disposable-emulator tests, explicitly
allows its software keyboard with a virtual hardware keyboard, and fails on any
app ANR. Generated offline video fixtures on Goldfish/Ranchu debug emulators use
Android's real `c2.android`/`OMX.google` software codec, avoiding the invalid
host-FFmpeg buffer bridge. Normal app/device codec selection is unchanged.
Emulator decoding therefore does **not** establish physical hardware decoding.

Visual review also exposed a genuine root text-color bug: MaterialTheme did not
supply LocalContentColor for a plain root Column, leaving headings/body black on
dark palettes. The root now supplies the shared foreground color explicitly.
Theme checks now verify foreground text layout color and painted background for
all 13 entries, including System, and reject a covering system ANR dialog.
Visible video keeps the screen awake only while playing; in-flight feed replies
retain the user's current scroll index. The full rerun remains required.

### Third run: all native UI regressions pass; video fixture timing remains

[Run 40660857974](https://github.com/Vanir20342222/ytui/actions/runs/40660857974)
at `7319021` passed **15 cases**, including all five native UI cases and all 13
background/foreground theme checks, with no covering launcher dialog. Video
visibility still failed; the public case was explicitly skipped. The trace
confirms Android's `c2.android.avc.decoder` was selected, and shows pause at
0.671 seconds before the visible TextureView was attached. The fixture now keeps
playing/repeating until the full screenshot contains actual RGB video pixels,
then tests pause/position/quality. Failures additionally save native view/texture
diagnostics. No visibility assertion is removed or replaced by decoder counters.

The follow-up also repairs pasted/saved playlist browse IDs (`VL`), preserves
Music/audio intent from the Downloads tab, keeps watch links with `list=` as
single items, and avoids treating another host's `/playlist` path as YouTube.
Successful empty-feed notices now translate desktop F2 instructions as well as
errors. Save toggles report removal accurately and refresh an open saved shelf.
Offline URL/notice regressions cover these changes; full runtime qualification
remains pending. Release Kotlin/Java compilation, 11 release JVM tests and release
lint also passed for the native UI; no signing identity was loaded for that check.

### Fourth run: visible video passes; concurrent export exposes a notification collision

[Run 40665889912](https://github.com/Vanir20342222/ytui/actions/runs/40665889912)
at `2c2d3a7` passed **16 cases**, including actual RGB video, pause/quality/position,
volume persistence, preview restoration, offline playback, all six native UI
checks, radio metadata, crypto and challenge vectors. One concurrent-export
playback case failed; the public case was explicitly skipped. The retained
screenshot now shows the generated color-bar video inside the native player.

The export worker used notification ID **4**, the same ID as playback. Publishing
and removing its non-media foreground notification replaced the MediaStyle entry;
System UI destroyed the active media session. Job notifications now have separate
preparation/export namespaces, away from playback/download IDs; the preparing
playback fallback also verifies its notification channel. A JVM regression and
an active-session/channel assertion extend the existing real export/decode test.
Debug/release JVM tests and lint pass locally; the full device rerun remains
required. This failure is retained, not dismissed as an emulator-only issue.

The full local workspace at this source passed **317 tests, 0 failures, 31
explicitly ignored**; strict Clippy passed. The six native UI cases are keyboard/
Unicode editing, all-theme background/foreground/persistence, protected-account
navigation/cancel, wide-layout panels, cooldown prefetch, and pasted URL intent.

## Passed historical backend checks

- Qualified Keystore/navigation checkpoint: **311 passed, 0 failed, 31 explicitly ignored**.
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
  [Rust run 40641207981](https://github.com/Vanir20342222/ytui/actions/runs/40641207981)
  passed desktop/shared checks, real FFmpeg/mpv/local-playlist checks and release
  build after the monorepo move. Desktop launchers/state paths are unchanged.
- Complete local Android build: ARM64 and x86-64 native libraries, dependency
  notices, Gradle wrapper, debug/instrumentation APK assembly, **8 Java unit tests**
  and Android Lint with **0 errors**. APK inspection passed signing, ZIP and
  **16 KiB ELF** alignment, both ABIs, JNI/GameActivity symbols and bundled notices.
- Accelerated API 35
  [Android run 40641208084](https://github.com/Vanir20342222/ytui/actions/runs/40641208084)
  passed **10 device tests, 0 failures, zero Rust panics**. The separate opt-in public YouTube case
  was explicitly skipped in this default, account-free run. Passed device checks:
  - Generated MP4 actually decodes into the Rust GLES surface and produces PCM
    levels; pause and volume persistence across stream changes work.
  - Short muted preview restores the previous media/position/preferences on EOS.
  - Actual loopback HTTP download plays from private offline cache after its
    server is closed.
  - Separate video/audio tracks export from cache into a playable MP4 with both
    tracks and decoded PCM. Foreground M4A export is byte-for-byte identical;
    cancellation does not create an output file.
  - Closing/reopening the Rust UI in the same process recreates the UI and leaves
    Android's main loop responsive.
  - JNI contracts/direct-buffer bounds and empty audio EOS input are checked.
  - Android's isolated JavaScript runtime has no browser/network/cookie APIs;
    original saved YouTube n/signature vectors return the exact expected maps.
  - Actual EOF album continuation, remote Previous/Next and pause/resume,
    history navigation, manual queue priority and failed-item retention pass.
  - JNI lyric save/load retains exact timestamps/Unicode and rejects invalid edits.
  - Actual Android Keystore encryption uses fresh nonces and rejects tampering.
    Rust/JNI import stores ciphertext, migrates a synthetic plaintext account,
    fails closed on a damaged envelope, recovers and removes it on disconnect.

The run's device-evidence artifact contains JUnit results, continuous logcat,
exit information and an actual 1080×2400 player screenshot. Visual inspection
confirmed system-bar separation and visible sliders/clock labels. It is generated
local test media, not evidence of public YouTube playback or audible output.

## Historical development APK (failed physical UI acceptance)

Qualified checkpoint: source `59675e0` (PR merge revision `b5c48f3`).
[APK and inspection reports](https://github.com/Vanir20342222/ytui/actions/runs/40641208084/artifacts/12795433805).
The ZIP contains `outputs/apk/debug/app-debug.apk` and `SHA256-and-checks.txt`.
The APK's SHA256 is:

```text
fc2654bcca9e7e845968200286bcd7c3cf6bdede83e0a2047eb8299d15c0d4ab
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
Run `40579287370` passed this entire export regression after the fix below.
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

The full local and hosted checkpoint passed **297 tests, 0 failures, 31 ignored**.
The focused mobile suite at that checkpoint passed **23 tests** and strict Clippy.
Safe-area contracts also include left/right display cutouts for phone landscape;
headless geometry checks cover all four edges. Physical rotation acceptance
remains separate.

## Playback continuation/history follow-up

Finite Music albums/playlists and cliamp track libraries now prepare their full
remaining collection in the background after the first decoder start. Manual
queue entries take priority. A new selection, disconnect or stopped service
invalidates obsolete collection work; pause/preview/reconnect cannot reset a
prepared tail. The derived context is not persisted after process death; explicit
queue entries and local playback history remain durable.

Previous/forward history commits only after playback starts, and writes stay in
decoder order across service/Activity recreation. Failed candidates stay queued.
The recently-played window can replay entries or clear history/resume positions
without deleting saved items or queued tracks. Android MediaSession navigation
forwards to these same controls.

New core tests cover manual priority, cached collection continuation, stale
responses, failed navigation, private context clearing and durable history. The
focused suite passes **27 tests**. Java compilation/unit tests/lint also pass.
The added emulator case exercises actual EOF continuation, MediaController
Previous/Next, forward history, manual priority and unavailable-track retention;
its result must pass before runtime qualification. Fixture activities now launch
with a debug-only offline UI flag to suppress ambient feed requests. Only the
explicit public test invokes real YouTube extraction.

Run `40586469283` passed EOF continuation and the initial controller Previous, but
controller Next disappeared after the source changed. A basic ForwardingPlayer
forwarded ExoPlayer's unmodified command-change events, overriding the augmented
getter. The adapter now uses Media3's ForwardingSimpleBasePlayer, which keeps
state and listener events consistent; the full controller regression remains.

Run `40588666075` passed the complete EOF/controller/history test. The stream
switch test then exposed an older notification race: each Play replaced the
active MediaStyle notification with a non-media preparing placeholder. System UI
removed the session and sent Stop after the new stream started. The provider is
now configured once; foreground entry reuses an existing notification or creates
a MediaStyle placeholder with the session token. Remote pause/resume also updates
the application's user intent, and remote Stop clears pending continuation.
These changes must pass the full device suite before qualification.

Run `40590717413` passed remote pause/resume, EOF, history and all other cases,
but rapid preview entry still triggered the System UI Stop race. Already-ongoing
playback now leaves Media3's foreground notification untouched; the deadline
placeholder is used only when the service is not already in foreground playback.
Run `40640008655` passed the entire regression, including rapid preview entry,
stream switches, remote pause/resume/Previous/Next and EOF continuation.

The cloud host lacks `/dev/kvm`; its software emulator was unreliable and an
earlier instrumentation attempt crashed. Those attempts are not counted as
passes. The accelerated hosted emulator supplied the successful runtime evidence.

## Acceptance still required

### Timed lyrics follow-up

The Rust lyric window replaces the old plain popup. It supports reviewed LRCLIB
title/artist lookup, version selection, virtualized timed follow and line seeking,
UTF-8 LRC/text import, text editing and per-media timing adjustment. A different
playing URL suspends timing/seeks; dismissed/superseded requests and imports
cannot reopen or overwrite another view. Local documents are limited to 512 KiB,
100 songs / 8 MiB total, and use a separate atomic private file so queue/resume
writes do not serialize the lyric library. No automatic lyric/history lookup is
sent. Tests cover Unicode/timestamps, offsets, all themes at phone/tablet sizes,
late replies, invalid edits, persistence rollback and library capacity. Android
document tests reject malformed/truncated UTF-8, excessive bytes and stuck streams.
An added JNI device case checks exact timings and failed-edit rollback. Its
emulator case passed in `40640008655`; live LRCLIB/device keyboard acceptance
remains separate.

### Keystore follow-up

Account storage now seals sessions before atomic persistence with Android
Keystore AES-GCM, fresh 96-bit nonces, authenticated app/version context and a
non-exportable 256-bit key. Legacy private files migrate; a failed cleanup is
reported. A bad/missing key fails closed without deleting the saved envelope or
breaking offline library access. Document parsing no longer needs a plaintext
temporary file. Focused Rust tests cover corrupt-account recovery and cookie
file bounds/pipes. New Android cases exercise actual Keystore randomization,
tamper rejection, Rust/JNI encrypted import, plaintext migration, damaged-envelope
recovery and disconnect using a disposable synthetic account only. They skip
instead of overwriting an existing account. Run `40641208084` passed both added
cases, the other eight device tests and the zero-panic gate. Only the public
YouTube test was skipped; the account tests ran against synthetic data.

Account actions also carry a monotonically increasing intent checked under the
commit lock. A delayed import cannot reconnect after a newer disconnect; stale
startup/import UI replies cannot overwrite the current account status.

### Signing and in-memory native auth follow-up (qualification pending)

The manual main-only signed-build workflow requires prior successful checks for
the same source revision and owner-supplied Environment secrets. Gradle rejects
missing signing material even through aggregate assemble tasks, validates version
metadata and never falls back to debug signing. A disposable local identity is
used to check the build path; it is not an owner release identity. About shows
version/code and source revision. No release/tag is published by this workflow.
The local signed build passed both Rust ABIs, eight release Java unit tests,
release lint, signature/16 KiB inspection, expected version metadata and the
non-debuggable-manifest check. Direct and aggregate release builds without a key
failed as expected; invalid version fields were rejected and normal debug builds
still configure without signing secrets. The disposable private key/password
were removed after verification; only public reports remain. The hosted signing
workflow still requires the owner's environment setup and is not yet exercised.

Mobile native extraction now builds its scoped cookie jar directly in memory,
removing the old transient Netscape export step. Startup removes only recognized
regular files in its own old staging directory, preserves unrelated files and
refuses symlinked staging directories. Host tests cover HTTPS/domain/path scope
and selective cleanup; a new full APK/runtime qualification remains required.

### Radio metadata and download follow-up (device qualification pending)

Live ICY song titles now update the player and notification while preserving the
station's library/history identity. Titles are bounded and strip control codes;
Unicode code points are not split. A generated MP3 with two actual HTTP/ICY title
updates is the new device regression; no external radio service is used.
The first run (`40645113719`) decoded audio but left the fixed station title in
place. MediaItem's metadata takes priority over timed ICY data. The adapter now
consumes `onMetadata` IcyInfo entries and overlays the live title in session state
without replacing/restarting the source. The regression also checks the session
title and ensures metadata-only tracks are not mislabeled as selectable video.

Download requests now retain their selected quality through asynchronous replies,
playlist preparation and failed-stream refresh. Download-index data drops unused
format/caption catalogs to reduce repeated parsing during library polling. Existing
offline requests skip external caption sources that were never cached; embedded
tracks remain available. The export device case checks metadata compaction,
retained quality and real cache-only decoding with legacy uncached captions.

### Physical/live release gates

On both requested physical devices: install/update signing identity, soft keyboard,
rotation, tablet split layout, gesture/system insets, accessibility, every
settings/modal screen, all themes/player styles, real video and Music playback,
codec/quality/track/caption selection, HTTP/HTTPS stations, SponsorBlock timing,
account expiry/disconnect, multi-page feeds/playlists, paired offline media,
network loss/VPN handoff/reconnect/rate-limit cooldown, process death, screen lock,
audio focus/headset/Bluetooth, notifications, background limits, measured
memory/CPU/frame latency and human audible confirmation. Advanced capability gaps
remain explicit in [PARITY.md](PARITY.md). M9 remains open.


## M9 startup latency work, 2026-10-29 (in progress)

[Research, implementation and acceptance target](TWO_SECOND_STARTUP.md) documents
mobile.5. The target is actual tap → visible native frame within 2000 ms, including
extraction. Five offline direct-player request-budget/fallback tests passed;
Android compilation/lint and 16 JVM tests passed. Hosted rebuilt-ABI runtime and
physical Tab S9 timing results are pending. A fresh anonymous cloud probe failed
extraction in 329 ms; this is an unsuccessful playback attempt, not target success.
