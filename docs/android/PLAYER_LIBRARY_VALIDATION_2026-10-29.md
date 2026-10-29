# Android player/library qualification — 2026-10-29

[Download the signed APK](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.4/ytui-0.2.0-alpha.12-mobile.4.apk) · [Release, checksum and inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.4)

Version **0.2.0-alpha.12-mobile.4**, code **4**, Android 8/API 26+, ARM64/x86-64. Source **`cf8046508b96962f1b5ba9efc3529c4427d64b9f`** includes PRs #21, #22 and #23. This is a development preview with a stable signing identity, not a production/parity claim.

```text
SHA256 4eeaa2145a0b7b08328d940e84c16e1375e481c859f48ac5127e3598301b05c4
```

## Delivered behavior

- The native video view receives per-surface frame events directly from ExoPlayer. A separate MediaSession adapter handles external metadata/controllers. This repairs black inline video and video disappearing behind fullscreen controls. Native Next/Previous retain queue policy; pause retains user reconnect intent. Landscape fullscreen and both exit routes reuse the decoder and restore visible inline video.
- Stream resolution reuses bounded account/network/quality-scoped clients and expiring leases. Public versioned player assets are cached. Next warms the selected candidate; visible playlist items work while the full collection prepares. Manual queues take priority, and acceptance of actual decoder starts governs history/queue writes. Related-media preparation is enabled by default outside collections, without online fallback for explicit offline media.
- Seeking is optimistic during touch and reconciles acknowledged positions. The focused song player is non-scrolling, with Vinyl, VU and Orbit, secondary controls, music-only transitions and reduced-motion handling. Disabled gestures also disable header dragging; icon minimization remains available. Covered catalogs leave accessibility traversal and the focused pane is announced.
- Settings have Appearance, Playback, Offline & storage, Video information, Network, Privacy and Account categories. Personal playlists have direct source chips; video metadata visibility is configurable. Timed success notifications are removed; failures remain in screen content.
- Sized memory/disk artwork caches retry transient failures twice with cancellation-aware backoff. Radio uses provider artwork or [documented genre photographs](RADIO_ARTWORK.md). Cold images still need their first fetch; no zero-network-latency claim is made.
- Offline, Playlists and Transfers views support durable local playlist editing and separately chosen Music/Video document-tree folders. Compatible completed media copy/mux to those destinations and retain cache fallback when a grant is unavailable. Mode/quality identities keep separate outputs; legacy caches remain discoverable. Download parallelism is configurable from one to four, default three.

## Executed checks

| Check | Result |
| --- | --- |
| [Main Android](https://github.com/Vanir20342222/ytui/actions/runs/40837593351) | **24 passed**, 0 failures/errors, 1 explicit opt-in public YouTube skip across 25 JUnit cases |
| Visible native UI | Keyboard/Unicode, account navigation, all 13 theme paints/persistence, phone/tablet cards and actions, square covers, three visualizers, disabled gestures and icon minimize |
| Actual media/storage | Inline/fullscreen/return RGB frames; decoded video behind visible controls; native play/pause/reconnect intent; adapter Next while collection is blocked under 3 seconds; queue/EOF/history, volume, ICY, split media, exports, real folder grant and cache-free local playlist EOF |
| Continuous logs | **0 app ANRs, 0 Rust panic markers** |
| [Main Rust](https://github.com/Vanir20342222/ytui/actions/runs/40837593137) | **328 default tests passed**, 31 explicitly ignored; formatting, strict Clippy, local-media integration checks and release build passed |
| Android release | Both Rust ABIs built locally; **14 release JVM tests**, compile and lint passed. Hosted debug JVM tests/build/lint passed |
| Signing/publication | Six synthetic boundary tests; certificate, source marker, package/version, ZIP/ELF 16 KiB alignment, JNI and notices verified. [Publisher](https://github.com/Vanir20342222/ytui/actions/runs/40839716818) republishes the privately signed APK without rebuilding/re-signing |
| Public download | Downloaded the public GitHub APK and matched the SHA256 above |

[Actual native gallery](../screenshots/android/README.md) · [Complete device artifact](https://github.com/Vanir20342222/ytui/actions/runs/40837593351/artifacts/12882181356)

A previous passing run missed the fullscreen toolbar black-frame bug because frame and control assertions were separate. Manual review found **zero** RGB pixels behind the controls. The final test requires both at once; the qualified main capture has more than 200,000 pixels in each tested colour band. TextureView is the currently qualified presentation; hardware/HDR/power measurements remain open.

One earlier PR Rust run failed an existing download-hook fixture. One hundred local repetitions of the full fixture suite and one job rerun passed; main passed without a rerun. The original failure was retained. Its OS cause was not captured, so no product fix is claimed; the follow-up assertion includes its job error for future diagnosis. A Windows-wrapper attribute mismatch also produced a false dirty worktree despite byte-for-byte equality; byte-preserving attributes address that tooling issue.

The first publisher attempt received an expired five-minute APK-transfer ticket. A fresh transfer succeeded; no app rebuild, key upload or checksum change was involved.

## Installation and signing

Open the APK from Android Files and install **ytui** (`dev.ytui.android.stable`). It installs alongside older **ytui Preview** and original development packages without deleting their private data. Account/settings/library are separate: sign in and choose folders once. Future stable-channel updates use this same certificate:

```text
B3:E9:F1:A7:0D:D6:59:78:84:D1:DB:BA:09:EA:FA:D3:D4:D7:3E:A6:2C:40:74:A9:B8:CE:C1:AC:1E:03:48:48
```

The private identity remains outside Git, with owner-only SSH-encrypted recovery. No GitHub signing-secret setup is needed. Only the finished APK went through the temporary transfer; that staging file was removed after public verification. [Recovery/update instructions](SIGNING.md).

## Remaining acceptance

The earlier opt-in [public YouTube run](https://github.com/Vanir20342222/ytui/actions/runs/40781371045) failed native extraction with invalid player responses and a probable hosted-IP block. The default skip and local fixtures do not erase that failure. **Real YouTube startup within three seconds is not established.** Do not import personal sessions into public CI or retry aggressively to hide that result.

Galaxy Tab S9/Nothing Phone (3a), audible output, real personalized/anonymous feeds, Google sign-in policy, hardware codecs/HDR/power, OEM background behavior, active public streams and physical in-place stable updates require [device acceptance](DEVICE_ACCEPTANCE.md). [Parity](PARITY.md) lists desktop-only and unimplemented mobile capabilities, including MP3/FLAC transcoding, segmented exports and advanced desktop editing/scheduling. M9 remains a preview until those relevant acceptance gates are met.
