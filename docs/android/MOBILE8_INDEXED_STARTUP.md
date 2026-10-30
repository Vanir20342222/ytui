# M9: indexed stream startup, mobile.8

## Actual mobile.7 device result

The owner's Samsung Tab S9 ran `1a6f9682d5c573f58992ced44aa9b7c6d82788d6`
with **Best** quality. Prepared video frames took 630, 74, 828, 146, 161 and
262 ms; prepared audio started in 148 ms. Two audio attempts remained
pending/cancelled. Cold video results were:

| Resolve | Handoff | Visible frame | Preparation-to-frame | Transport |
| ---: | ---: | ---: | ---: | --- |
| 561 | 589 | **4117** | 3509 | Progressive |
| 923 | 930 | **2948** | 1953 | Progressive |
| 867 | 872 | **2827** | 1885 | Progressive |
| 344 | 382 | 1165 | 767 | Progressive |
| 579 | 614 | 646 | 16 | HLS |

All times are milliseconds from the tap except preparation-to-frame, which is
relative to handoff. The legacy `decoder_ms` label includes buffering/source
preparation; it is not isolated hardware decoding time. Extraction now takes
338–917 ms, while the three slow progressive starts spend 1885–3509 ms after
handoff. First network byte was 273–403 ms after handoff. The final HLS result
rendered early but reported ready 2519 ms after handoff: a frame alone does not
prove usable playback. The live two-second goal remains **failed**, not complete.

## Research and implementation

NewPipe's [progressive DASH creator](https://github.com/TeamNewPipe/NewPipeExtractor/blob/6faf40681eff7da6c7a45fb92e3eb633a60a062b/extractor/src/main/java/org/schabi/newpipe/extractor/services/youtube/dashmanifestcreators/YoutubeProgressiveDashManifestCreator.java)
uses the extractor's initialization/index byte ranges in a DASH SegmentBase
presentation. Its [player data-source factory](https://github.com/TeamNewPipe/NewPipe/blob/c32810362e0b7ca192c3ee9b77ca83e81ca8b02d/app/src/main/java/org/schabi/newpipe/player/helper/PlayerDataSource.java)
uses query-range semantics for DASH CDN requests. These are inspected design
patterns, not measured NewPipe performance claims or copied implementations.

The Rust playback extractor now retains initialization/index ranges and stream
duration from the player response, without fetching media. The mobile engine
validates those bounds and creates an in-memory DASH presentation for the exact
selected adaptive tracks. Media3 reads initialization and indexed segments
instead of treating the resource as an unstructured progressive file. Selected
quality, codecs, dimensions and original media URLs are preserved; **Best is
never silently downgraded**. Missing/unsafe indexes keep the original source.
No generated manifest or signed media URL is persisted in diagnostic files.

A CDN adapter uses finite `range=start-end` queries only for HTTP(S)
`googlevideo.com/videoplayback` resources and preserves original global cache
positions/keys. It avoids double-skipping a query-ranged HTTP 200 response,
checks the returned length and retries the original header-range request when
the query is ignored or rejected. Signed `range` arguments and explicit caller
Range headers are left alone; rate limits/connectivity failures are not retried
by this adapter. Offline downloading/export still uses original resources.

Video-only HLS variants also retain codec declarations when paired with
separate audio. An indexed audio-only presentation can accompany that unchanged
HLS source. Unknown-codec/unsupported sources retain ordinary preparation.

The timing report adds actual `started` and `usable_start` values. Usable start
requires **both** visible output and active playback; an early frame followed
by buffering/failure cannot pass. A queued TextureView update from the previous
item is ignored until the current item's renderer has produced its first frame;
renderer events are matched to their media ID and monotonic handoff time.
Numeric codec-initialization events are
recorded separately. Source failures and stop/replacement after handoff now
record their terminal outcome rather than remaining pending indefinitely.
Reports remain identifier/URL/header free and bounded.

## Qualification and remaining gate

Local Rust/Android checks and a synthetic indexed media contract exercise the
actual manifest builder. Native instrumentation checks cold indexed playback
and actual window pixels within 2000 ms, seeking, exact query-range offsets,
cache identity, ignored queries, rejected queries and signed-range fallback.
The fixture deliberately delays unbounded whole-file reads beyond the target;
bounded initialization/index/segment reads must be used. Existing catalog-tap,
HLS/cache, Next, fullscreen, UI/account/themes and offline checks remain required.

Qualification and signing results are recorded below. These checks cannot
establish live YouTube speed. The cloud's normal
YouTube CONNECT is denied with HTTP 403; no proxy/routing workaround is used.
After APK qualification/publication, the owner must retest on the Tab S9 with
Best, several new videos and Next and send the complete timing report including
`usable_start` and failed/cancelled attempts. M9 and live acceptance remain open.

The first [PR native run](https://github.com/Vanir20342222/ytui/actions/runs/41062613942)
failed the indexed startup case and a subsequent existing video case. The indexed
failure is a real integration bug: Media3's DASH CMCD handling calls hierarchical
URI APIs on the opaque `data:` manifest URI. It now uses a bounded request-local
byte-array source with hierarchical `ytui-memory://` URIs. No manifest network
request is added; resources/subtitles keep the normal source and cache. Failed
test playback is stopped in `finally` to avoid contaminating later cases. A fresh
full native qualification is required; neither deadline nor pixel checks changed.

## Qualified source and signed artifact

App source: **`bd75363c0098edcdc54840c9cb7c7a65aebc09ef`**, merge of
[PR #28](https://github.com/Vanir20342222/ytui/pull/28). Version code **8**,
name **0.2.0-alpha.12-mobile.8**, package `dev.ytui.android.stable`, Android 8+,
ARM64/x86-64. The signing certificate matches mobile.4–7; installation should
update the existing stable app without uninstalling or deleting its state.

[Download mobile.8 APK](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.8/ytui-0.2.0-alpha.12-mobile.8.apk)
· [Release and public inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.8).

- [Final PR Android](https://github.com/Vanir20342222/ytui/actions/runs/41065115995): **32 pass, one public opt-in skip, zero failures**. Indexed usable video **1057 ms**, actual window pixels **1356 ms**, seeking passes. Exact query-range bytes, original cache identity, ignored/rejected/signed queries, source failures, stop cancellation and stale-frame rejection pass.
- [Exact-main Android](https://github.com/Vanir20342222/ytui/actions/runs/41067553848): **32 pass, one public opt-in skip, zero failures**. Indexed usable video **914 ms**, window pixels **1585 ms**. Catalog-tap window pixels **1229/1105 ms** cold/prepared; HLS window pixels **738/798 ms**. All original native UI/themes, fullscreen, queue/Next and offline/export checks pass; no app ANR or Rust panic markers.
- [Exact-main Rust](https://github.com/Vanir20342222/ytui/actions/runs/41067554096): **336 workspace + 15 extractor TLS + 15 solver/media checks** pass (366 total, 31 explicit ignores), formatting and strict Clippy pass.
- [Secret scan](https://github.com/Vanir20342222/ytui/actions/runs/41067553908) passes. Full-history local scans pass without new exemptions. Both release ABIs, **19 release JVM tests**, lint, source/package/version/certificate, ZIP/16 KiB alignment and notice inventory checks pass.

The initial push-only [Rust run](https://github.com/Vanir20342222/ytui/actions/runs/41062605617)
failed the cancellation fixture's ready assertion. The parallel PR run, both
final-head runs, exact main and 50 isolated local repetitions pass with the
unchanged two-second cancellation deadline. Its cause was not reproduced; this
is retained as an intermittent fixture failure, not a diagnosed production fix.

Signed APK SHA256:
`556bd3e8190022c2c8fb3002d9b9fbba42d298a158fa97e3029576bc9bdfc06e`.
Size: **56,090,566 bytes**. Certificate SHA256:
`b3e9f1a70dd6597884d1dbba09eafad3d4d73ea62c4074a9b8cec1ac1e034848`.

The [publisher](https://github.com/Vanir20342222/ytui/actions/runs/41069978120)
passes, transferring only the finished signed APK without receiving credentials,
rebuilding or re-signing. The public APK was independently downloaded and its
hash, size, certificate, package and code 8 verified. The temporary private APK
transfer and local bearer inputs were deleted; the permanent identity and
encrypted owner recovery remain private. The owner has been asked to retest
Best, new videos and Next and send the complete usable-start report. Live
two-second acceptance and M9 remain **open**.

## Further checks

The existing opt-in public instrumentation is a 720p smoke/budget check. It is
not Best-quality acceptance; the Tab S9 must use Best and the new usable-start
report. The live test is skipped here because normal YouTube CONNECT is denied.

Inspection also finds that visible-frame callbacks still publish full player
snapshots after the first frame. Throttling that post-start work deserves a
separate measured change; no unmeasured performance benefit is claimed here.
