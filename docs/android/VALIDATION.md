# Android validation record — 2026-10-27 (M9)

This record distinguishes implemented code from qualified behavior. Android is a
preview. No physical Nothing Phone (3a) or Galaxy Tab S9 has been connected.
No personal cookies, browser database, VPN or desktop appearance settings were
used or changed by these tests.

## Evidence already collected in the cloud workspace

- Existing desktop/shared offline workspace suite after initial provider/desktop
  move: **282 passed, 0 failed, 31 ignored**. The final expanded suite must also
  pass after all M9 follow-up changes; its result is recorded below when run.
- Expanded focused mobile suite: **13 passed, 0 failed** at the current checkpoint:
  all built-in theme entries across 360×800, 412×915 and 1280×800 logical layouts;
  four tabs/overlays; touch navigation; Back overlay order; stale playback/account
  reply rejection; bounded feed cooldown; scoped synthetic account import and
  disconnect; failed-write rollback; durable saved/queue/resume; private palette
  import; malformed requests/config rejection. Render checks are headless Rust
  UI checks, not screenshots or device performance measurements.
- Android Gradle debug APK and instrumentation APK assembly passed. Android Lint
  passed with **0 errors**; warnings retain explicit pinned-dependency upgrade
  notices rather than blanket suppression.
- Rust strict workspace/all-target Clippy passed at an earlier checkpoint;
  the final follow-up gate is run again for changed code.
- ARM64 and x86-64 Rust GameActivity native libraries built successfully at the
  initial checkpoint, including JNI and GameActivity entrypoints. A final build
  and APK alignment/signature inspection is required for the packaged revision.
- A prior APK installed and displayed the Rust UI in an API 35 software emulator.
  This exposed search-row overflow and system-inset overlap, corrected in source.
  The final GameActivity keyboard/insets changes require fresh visual acceptance.

## Failed / blocked checks, retained honestly

The cloud host has no `/dev/kvm`. The software emulator had Android launcher,
Bluetooth and Digital Wellbeing failures/ANRs, very slow cold starts, and an
instrumentation attempt ended **“Process crashed”**. That is not an instrumentation
pass. The only captured Android crash-buffer entries at that checkpoint belonged
to Bluetooth; the app failure cause remains unresolved until the accelerated CI
run supplies logs. Do not infer that the app itself is sound from that observation.

The emulator's public YouTube request failed DNS/network access in this proxied
cloud environment. No public Android YouTube/Music extraction, personalized feed,
audible playback or actual public download is claimed as passed. Desktop prior
local-machine acceptance does not qualify the new Android adapters.

## Automated acceptance added

The Android workflow builds both ABIs, runs Java unit tests and lint, checks APK
signing/ZIP alignment/16 KiB ELF segments/JNI entrypoints/licenses, and uploads a
sideloadable **development** APK plus reports. The accelerated API 35 job runs:

- Real JNI initialization, request/error contracts and direct-buffer bounds.
- AndroidX isolated JavaScript API isolation and saved upstream YouTube
  n/signature challenge vectors (no public network or real session).
- Generated MP4 actual decoder progress into the Rust video surface, pause,
  volume persistence across stream changes, loopback HTTP download and playback
  from the private cache after the fixture server is closed.

These checks have been added to the repository; their CI results must be read
before describing them as passed. They do not replace real provider acceptance.

## Device acceptance still required

On both requested devices: install/update signing identity, search soft keyboard,
rotation, tablet split layout, gesture/system insets, every settings/modal screen,
all themes/three player styles/reduced motion, real video and Music playback,
codec/quality/track/caption selection, HTTP/HTTPS stations, SponsorBlock marks/skip/
replay, account expiry/disconnect, multi-page feeds/playlists, paired offline media,
network loss/VPN handoff/reconnect/rate-limit cooldown, process death, screen lock,
audio focus/headset/Bluetooth, notifications, background limits, memory/CPU/frame
latency and human audible confirmation. Advanced parity gaps are explicit in
[PARITY.md](PARITY.md). M9 remains open.
