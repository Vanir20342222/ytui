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
by buffering/failure cannot pass. Numeric codec-initialization events are
recorded separately. Reports remain identifier/URL/header free and bounded.

## Qualification and remaining gate

Local Rust/Android checks and a synthetic indexed media contract exercise the
actual manifest builder. Native instrumentation checks cold indexed playback
and actual window pixels within 2000 ms, seeking, exact query-range offsets,
cache identity, ignored queries, rejected queries and signed-range fallback.
The fixture deliberately delays unbounded whole-file reads beyond the target;
bounded initialization/index/segment reads must be used. Existing catalog-tap,
HLS/cache, Next, fullscreen, UI/account/themes and offline checks remain required.

Build, emulator, signing and publication results will be recorded here after
they run. These checks cannot establish live YouTube speed. The cloud's normal
YouTube CONNECT is denied with HTTP 403; no proxy/routing workaround is used.
After APK qualification/publication, the owner must retest on the Tab S9 with
Best, several new videos and Next and send the complete timing report including
`usable_start` and failed/cancelled attempts. M9 and live acceptance remain open.
