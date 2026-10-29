# M9 startup performance preview: mobile.5

Source `7d5267af5b360d211fe8f7e33d21e0ec21448e59`, merge of [PR #24](https://github.com/Vanir20342222/ytui/pull/24).

## Reproducible evidence

- [Exact-main Rust run](https://github.com/Vanir20342222/ytui/actions/runs/40910296725): 329 workspace tests pass, 31 explicit ignores; eight mobile extractor TLS cases pass, plus 15 solver/media checks. Formatting and strict Clippy pass.
- [Exact-main Android run](https://github.com/Vanir20342222/ytui/actions/runs/40910296881): 26 native runtime cases pass, no failures; one public opt-in case is explicitly skipped. Six alternating service restarts, all themes, keyboard/account navigation, offline folders and native/fullscreen video controls pass. No app ANR or Rust panic markers occur in the successful run.
- [Exact-main secret scan](https://github.com/Vanir20342222/ytui/actions/runs/40910296780): passes. The full history scan is clean; signing credentials remain outside the repository.
- The privately signed release builds both ARM64/x86-64 engines; 16 release JVM checks and lint pass. APK inspection verifies stable signature, package/version, source marker, ZIP/16 KiB ELF alignment and bundled notices.

The startup test taps a real catalog card, resolves through Rust, starts Media3
and requires RGB pixels copied from the actual focused activity window within
2000 ms. It uses generated loopback media, not YouTube. A poster, decoder-only
counter or playing flag does not pass. Cold/prepared resolution-cache states are
separate. Pixel-copy completion is a conservative presentation upper bound;
view-frame events are reported separately.

Exact-main window-copy completion: **1921 ms cold / 712 ms prepared**. Native view-frame events: 446/368 ms. Both pixel deadlines are unchanged at 2000 ms.

PR qualification independently confirmed actual window pixels at 1476 ms cold
and 1251 ms prepared (view events 567/817 ms). Earlier failed runs remain recorded
in [the research/implementation log](TWO_SECOND_STARTUP.md), including the
notification ownership race and animated native-view compositing defects.

The first exact-main emulator attempt was not qualification: Google's setup app
`com.google.android.googlesdksetup/.DefaultActivity` had an input-focus ANR during
boot. Its system modal covered the decoded video and subsequent keyboard/theme/
folder tests. The failed video and startup screenshots both show that modal;
ytui's exit records contain no app ANR. The same-source fresh-emulator rerun keeps
all assertions and the 2000 ms deadline unchanged. [Failed attempt evidence](https://github.com/Vanir20342222/ytui/actions/runs/40910296881/attempts/1)
is retained rather than reported as a pass. A subsequent test-harness change
excludes only this first-boot setup package on emulators, before instrumentation;
it changes no APK application source, does not run on physical devices and keeps
all application ANR/system-dialog assertions. Its fresh CI result is separate from
the published APK's exact-source qualification above.

## APK and updates

[Download mobile.5](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.5/ytui-0.2.0-alpha.12-mobile.5.apk)
· [Public checksum and inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.5).
Android 8+, package `dev.ytui.android.stable`, version code 5, ARM64/x86-64.
The certificate matches mobile.4; install over the stable **ytui** app. No
uninstall or GitHub signing-secret setup is required. Old **ytui Preview** is a
separate package and is not updated by this APK.

SHA256: `446bfb12127ad4e001024adb0ba0c38afa4e2441b48353ff9703572942938dfa`.
Size: 55,803,606 bytes. The public APK is independently downloaded and checked
against the local signed artifact before this qualification is published.

## Live gate remains open

The configured cloud proxy rejects YouTube CONNECT before TLS (HTTP 403).
No alternate network route or account credentials are used to bypass it. Offline
fixture speed cannot qualify live YouTube extraction, signed media retrieval,
audible output or physical-device/OEM behavior. The owner agreed to test on a
Galaxy Tab S9. M9 and a universal two-second live claim remain unqualified.

On the Tab S9, install the APK, play several new videos, try Next, and copy
**Settings → Performance → Copy timing report**. Include cold and prepared
attempts and any failures; do not select only fast samples. Reports retain at
most 20 attempts in memory, include pending/cancelled attempts, allowlist route
labels, and contain no video IDs, titles, signed URLs, cookies or account data.
Copying is explicit and nothing is uploaded automatically. The target includes
resolution through the first visible native frame, not just decoder startup.
