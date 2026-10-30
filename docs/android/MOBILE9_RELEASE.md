# M9: mobile.9 UI release

Version **0.2.0-alpha.12-mobile.9**, code **9**, packages the reviewed UI from
[PR #27](https://github.com/Vanir20342222/ytui/pull/27) on app source
**`553fe494b800d472de239285391c6643cc97e246`**. Android 8+, ARM64/x86-64,
package `dev.ytui.android.stable`.

[Download mobile.9 APK](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.9/ytui-0.2.0-alpha.12-mobile.9.apk)
· [Release, checksum and inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.9).

The search input is centered and capped on wide layouts, chip rows share 48dp
targets, artwork preserves leading-edge captions, VU meters are grouped with
the cover and avoid corner clipping, and About copy is less crowded. The review
also corrected modifier constraint order and bounded narrow VU spans.
[Review findings and original failed run](UI_PR27_REVIEW.md) are retained.
Indexed Best-quality playback and caching from mobile.8 remain included.

## Exact-source qualification

- [Android](https://github.com/Vanir20342222/ytui/actions/runs/41074393192): **33 native passes, one public-network skip, zero failures**. The portrait/landscape geometry regression, native input/account navigation, all themes, player styles, fullscreen, queue and offline/export pass. No app ANR or Rust panic markers.
- Strict actual window pixels: indexed **1532 ms**, catalog cold/prepared **1298/1222 ms**, HLS cold/prepared **1476/831 ms**. No startup deadline changed. Native evidence artifact **12986764105** belongs to that exact main revision.
- [Rust](https://github.com/Vanir20342222/ytui/actions/runs/41074393185): **366 workspace/extractor/solver/media checks pass**, 31 explicit ignores, formatting and strict Clippy pass.
- [Secret scan](https://github.com/Vanir20342222/ytui/actions/runs/41074393331) passes. The separately signed release build passes **19 release JVM tests**, lint, package/version/source/certificate, both ABI and ZIP/16 KiB alignment/notice inventory checks.

The owner's six mobile.8 Best-quality Tab S9 samples pass the observed video
target: five cold usable starts **1115–1752 ms**, one prepared **632 ms**.
[Physical table](MOBILE8_INDEXED_STARTUP.md). Mobile.9 adds UI changes; its new
physical-device results and broader M9 service/audio/hardware gates remain
separate from generated-media qualification. This is a development preview.

## Build metadata and signing

The existing version inputs package the already-qualified main source as the
next update. Exact-source native CI uses repository-default Preview metadata;
release code/name 9 are separately built and inspected using:

```bash
export YTUI_ANDROID_VERSION_CODE=9
export YTUI_ANDROID_VERSION_NAME=0.2.0-alpha.12-mobile.9
python3 tools/android/sign-local.py /private/path/to/recovered-identity
```

Keep identity files outside Git, owner-only, and recover them as described in
[SIGNING.md](SIGNING.md). The script never prints credentials. These supported
overrides are necessary to reproduce the release version from its source tag;
the tag's default Preview metadata is earlier. Future public updates must use
an increasing version code and the same permanent identity.

The certificate matches mobile.4–8. Install over the stable-channel app to retain
account, settings and offline library; no uninstall is needed.

## Publication

The [publisher](https://github.com/Vanir20342222/ytui/actions/runs/41079072261)
passes, transferring only the finished signed APK without credentials, rebuilding
or re-signing. The public APK was independently downloaded; its hash, size,
certificate, package, version name and code 9 match the signed build. Temporary
private APK transfer and bearer input files were deleted. The permanent identity
and encrypted owner recovery remain private.

Signed APK SHA256:
`d6dbcb84cce99decdfd6d5e801572e01bf4342545c79f3bf471440ac1da787a5`.
Size: **56,090,566 bytes**. Certificate SHA256:
`b3e9f1a70dd6597884d1dbba09eafad3d4d73ea62c4074a9b8cec1ac1e034848`.
