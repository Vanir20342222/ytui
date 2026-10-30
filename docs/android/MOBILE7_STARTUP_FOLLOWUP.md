# M9: Best-quality HLS startup follow-up, mobile.7

## Physical failure retained

The owner's Tab S9 ran mobile.6 (`97e0b7`) with **Best** quality. Five completed
video starts took **8713, 6772, 9431, 11048 and 9393 ms** to a visible frame.
Three further starts were cancelled at 2860, 1692 and 6478 ms. The live
two-second goal remains failed; mobile.6's loopback results do not supersede it.

The player API itself took 118–189 ms. Total extraction took 3936–8693 ms, one
request joined a flight for 6006 ms and another waited 2888 ms for capacity.
After handoff, the first frame still took 1813–2560 ms. The report excludes
video IDs and private URLs. Best quality was confirmed separately by the owner.

## Findings and implementation

Mobile.6 avoided optional manifests only when a direct stream already reached
the configured cap. Best correctly continued discovering higher qualities.
However, the shared HLS parser then fetched **every variant and audio rendition's
media playlist serially**, collecting all segment lists. Media3 fetched the
chosen playlist again. A fast player API therefore did not imply fast resolution.

Mobile playback now parses the bounded master quality/codec/language catalog
without traversing children. The existing selector still picks the same quality,
including Best and external audio groups. Media3 validates and loads the chosen
media playlist and segments. Full native fallback uses this traversal policy too.
Normal CLI/download extractor traversal still acquires segment lists; its
regression explicitly requires them. Android's durable Media3 downloader follows
its selected original URLs as before.

For a selected muxed HLS variant with known video/audio codecs, the engine also
supplies a small in-memory single-variant master retaining its CODECS declaration.
Starting with only a bare media playlist lost that declaration and required
segment sniffing to discover tracks. Media3 can now use chunkless preparation
without another master request. Original media URLs, quality selection, offline
cache identities, paired-source fallback and caption catalogs remain intact.
Unknown-codec and external-audio pairs keep the existing preparation path.

The [NewPipe source factory](https://github.com/TeamNewPipe/NewPipe/blob/c32810362e0b7ca192c3ee9b77ca83e81ca8b02d/app/src/main/java/org/schabi/newpipe/player/helper/PlayerDataSource.java)
also enables HLS chunkless preparation. This is an inspected design pattern,
not a claim about measured NewPipe performance or copied implementation.

Speculative byte writers pause at a foreground tap, preserving stream extraction
that the tap can join and completed cached bytes. A stale viewport response
cannot start a new prefix writer after that tap. Reports add numeric manifest,
first network response and first network byte stages, plus allowlisted quality
and transport labels. Network stages are relative to handoff and include selected
playlist requests; they are not claims of first segment data or audible output.

The HLS preloader previously cached playlist text alone. It now warms a bounded
first segment prefix (and a bounded initialization segment when present), using
the same URI/byte-range cache identities as playback. Masters/live playlists are
not recursively expanded or marked byte-ready. A native prepared-HLS check
requires actual segment bytes in cache and no second first-segment HTTP request
when playback starts; playlist-only warming cannot pass it.
Relative init/segment references use the response's final playlist URI after
redirects, including the redirect metadata retained by the cache.

## Qualification

Fourteen offline TLS regressions cover direct request budgets, Best quality,
manifest-only HLS, external audio groups, normal fragment traversal, quality
preservation, visitor reuse, fallback and rate limits. Core tests cover safe
codec-master construction and unchanged unsupported/paired paths. An actual
Android HLS service test requires a visible frame within 2000 ms, pixel
corroboration and requests limited to the selected playlist/segments. Existing
tap-to-pixel, fullscreen, queue, offline, input and theme checks remain required.

Build/native/signing/publication evidence is recorded below.
These generated-media checks cannot qualify live YouTube speed. The owner must
install the resulting APK over the stable app, keep Best selected, play new
videos and Next, and send Settings → Performance → Copy timing report. Include
slow/failed/cancelled attempts. M9 and live two-second acceptance stay open.

The first [PR native run](https://github.com/Vanir20342222/ytui/actions/runs/41026984712)
failed four cases and is retained as failed evidence. Three loopback-download
fixtures depended on the emulator's public-Internet validation despite serving
real files on localhost. They now temporarily remove that scheduler requirement
inside the disposable test install and restore it afterward; production policy
is unchanged. The HLS test recorded a 334 ms view-frame event but its immediate
full-display screenshot check failed. It now uses the same overlay-aware window
pixel readback as the existing startup tests, still with a strict 2000 ms pixel
deadline. A fresh native pass, including prepared-segment reuse, was required and is
recorded in the qualified artifact section below.

The second [PR native run](https://github.com/Vanir20342222/ytui/actions/runs/41029222182)
confirms `requirements=1 unmet=1` before the loopback scope and passes the real
downloads/exports. HLS passes at 718 ms cold / 689 ms prepared window pixels,
including first-segment cache reuse. It still fails the existing initial-video
case: the emulator logs 902 invalid zero color-buffer migrations and displays a
black TextureView despite continued decoding. The offline Goldfish/Ranchu debug
renderer now avoids placeholder/detached surfaces and requests Media3's surface
recreation workaround. Physical devices and release builds retain the standard
renderer. No pixel check or startup deadline is removed; another full native
qualification was required and passed before publication, as recorded below.

## Published and qualified artifact

[Download mobile.7 APK](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.7/ytui-0.2.0-alpha.12-mobile.7.apk)
· [Release and public inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.7).
Qualified source `1a6f9682d5c573f58992ced44aa9b7c6d82788d6`, version code 7,
package `dev.ytui.android.stable`, Android 8+, ARM64/x86-64. Its permanent
certificate matches mobile.4–6; install over the stable app without uninstalling.

- [PR #26 native qualification](https://github.com/Vanir20342222/ytui/actions/runs/41031614148): 28 pass, one explicit public skip, zero failures. Window pixels 1223/1203 ms; HLS pixels 1350/837 ms; no app ANR/Rust panic markers.
- [Exact-main Android](https://github.com/Vanir20342222/ytui/actions/runs/41033508507): 28 pass, one public skip. Cold/prepared catalog-tap window pixels 848/996 ms (view events 567/691 ms); HLS service window pixels 704/626 ms (view events 501/379 ms). Actual segment-cache reuse, foreground JNI admission, native controls/fullscreen, offline folders/exports, all themes and keyboard/account navigation pass. No app ANR/Rust panic markers.
- [Exact-main Rust](https://github.com/Vanir20342222/ytui/actions/runs/41033507062): 333 workspace, fourteen extractor TLS and fifteen solver/media checks pass (362 total, 31 explicit ignores); formatting and strict Clippy pass.
- [Secret scan](https://github.com/Vanir20342222/ytui/actions/runs/41033507060) passes. Both release ABIs, 18 release JVM tests and lint pass. Signature, source/version/package, ZIP/16 KiB alignment and notices checks pass.
- [Publisher](https://github.com/Vanir20342222/ytui/actions/runs/41035226126) transfers the signed APK without rebuilding/resigning or receiving credentials. The public APK was downloaded independently and its hash, size, certificate and code 7 verified. The temporary private APK transfer and local bearer input are deleted; the permanent identity and encrypted owner recovery remain private.

APK SHA256: `0cfee4793404377a045bf9f0b78b63e060e1fc89dabac17bf2f39d29bf4fdf51`.
Size: **56,009,238 bytes**. Certificate SHA256:
`b3e9f1a70dd6597884d1dbba09eafad3d4d73ea62c4074a9b8cec1ac1e034848`.

The successful native run uses the documented offline emulator renderer profile.
These generated-media results do not qualify live YouTube speed. The mobile.7
Tab S9 timing report with Best quality is still required; live two-second
acceptance and M9 remain open.
