# M9: two-second tap-to-visible-video target

Measure from the actual tap until the attached native view presents its first
decoded frame. Extraction, offline lookup, service initialization, source
preparation and decoding all count. A poster, playing flag or decoder-only
counter cannot satisfy this target. Report cold and prepared starts separately.
The user will run the performance APK on the Galaxy Tab S9; hosted YouTube
requests were previously blocked, so fixtures cannot qualify live service speed.

## Primary-source research (2026-10-29)

| Source | Observed design | ytui action |
| --- | --- | --- |
| [NewPipe extractor](https://github.com/TeamNewPipe/NewPipeExtractor/blob/6faf40681eff7da6c7a45fb92e3eb633a60a062b/extractor/src/main/java/org/schabi/newpipe/extractor/services/youtube/extractors/YoutubeStreamExtractor.java) | Requests the VisionOS player response directly; other page information is separate | Try a mobile playback-only direct player route before the full watch-page/multi-client route; preserve fallback, account and format rules |
| [NewPipe source manager](https://github.com/TeamNewPipe/NewPipe/blob/c32810362e0b7ca192c3ee9b77ca83e81ca8b02d/app/src/main/java/org/schabi/newpipe/player/playback/MediaSourceManager.java) and [info cache](https://github.com/TeamNewPipe/NewPipe/blob/c32810362e0b7ca192c3ee9b77ca83e81ca8b02d/app/src/main/java/org/schabi/newpipe/util/InfoCache.java) | Bounded expiring info cache, current/neighbor priority and debounced preparation | Warm a bounded settled viewport, prioritize actual taps, cancel stale requests and retain account/network/quality isolation |
| [InnerTune playback](https://github.com/z-huang/InnerTune/blob/bfba5ecb16795cfc93ecce73d94ca583bcec917c/app/src/main/java/com/zionhuang/music/playback/MusicService.kt) | Checks media cache before resolution; uses stable media IDs, bounded chunks and defers metadata recovery | Reuse prepared leases/bytes and keep optional work off the first-frame path; retain ytui's stricter signed-URL expiry validation |
| [LibreTube player service](https://github.com/libre-tube/LibreTube/blob/fb0fbfd77028edeab3b6cb743b6b3d91c5348560/app/src/main/java/com/github/libretube/services/OnlinePlayerService.kt) | Separates stream acquisition/player initialization; supports DASH/HLS/SABR sources | Avoid blocking startup on unrelated metadata; SABR is a separate larger compatibility project, not a quick latency switch |
| [Media3 preloading](https://developer.android.com/media/media3/exoplayer/preloading-media/preloadmanager/concepts) | Prepared sources and sample buffers can be handed to a player sharing the builder/looper | Preserve correct source ownership; prioritize active playback and bounded preparation rather than reducing buffers indiscriminately |

These are inspected implementation patterns, not measured claims that those
projects meet a universal two-second bound. Implement original ytui code; do not
copy unrelated expiry, networking or session assumptions.

## Implementation and acceptance

1. Add end-to-end, local-only startup timing with stage breakdown and an explicit
   visible-frame event; expose a sanitized copyable report for the Tab S9.
2. Remove avoidable cold extraction requests with a mobile-only direct player
   route and robust full-extraction fallback; default CLI/download semantics stay.
3. Prepare bounded visible candidates on unmetered connections, reserve foreground
   extraction capacity, and warm safe infrastructure before playback. No history,
   queue acceptance or unsolicited playback occurs during speculation.
4. Test cold/prepared real native UI taps, cancellation/account changes, paired
   sources and native controls. Add an opt-in live benchmark that includes actual
   resolution and visible playback and fails its two-second budget honestly.
5. Build/sign the performance APK, pass runtime/regression/security checks, then
   collect real Tab S9 timings. Do not mark live two-second acceptance complete
   until actual measurements satisfy it; preserve failures and slow samples.

## First implementation (performance preview mobile.5)

- Mobile native extraction tries one direct VisionOS player API request and
  validates the configured format selector before returning it. Restricted,
  mismatched, challenged or unavailable responses use full acquisition. HTTP 429
  retains its error/Retry-After and does not trigger more client requests.
  The desktop/download option stays disabled. Extractor clients include the
  playback selector's audio/quality scope so an old selector cannot be reused.
- A settled browse row prepares up to three stream leases, with a 400 ms debounce,
  no playback/history/queue writes, no live stations and no default metered-data
  use. The first candidate also warms a bounded byte prefix. Active playback and
  actual tap resolution take priority. Stale prefetch cancellation reaches Rust;
  background extraction can occupy at most one of the two extractor slots even
  while a cancelled blocking task finishes.
- Streaming prefix writes now use the same custom cache keys as playback, and
  prefetch identity includes quality, audio, account and network. Next warming
  waits for the active decoder's initial buffer instead of competing immediately.
- The isolated JS sandbox connection warms before a tap; it does not evaluate
  scripts or load credentials/network data during warm-up.
- Settings → Performance exposes at most 20 in-memory, identifier-free samples.
  Each sample includes tap-to-resolution, service handoff and output. Video output
  is counted by the attached, shown native PlayerView's first-frame event; audio
  is labeled playing, not audible. Fullscreen does not overwrite the first result.
  Copying a report is explicit; nothing is uploaded automatically.
- The default device suite includes real cold/prepared catalog taps with a strict
  2000 ms visible-frame budget plus RGB screenshot corroboration. The opt-in
  public YouTube test includes extraction in its same 2000 ms budget. Offline
  success does not satisfy the live Tab S9 gate.

Reproduction: `cargo run -p ytui-mobile --example startup_probe -- PUBLIC_YOUTUBE_URL`
performs an explicit anonymous cold/cached resolution probe using temporary app
state. It prints sanitized timing/failure categories only. This is not a decoder
or visible-frame benchmark and is not run by default tests.

### Local and hosted evidence (qualification in progress)

The six offline direct-player tests passed locally (paired 1080p, audio,
restricted-response fallback, no extra request after 429, unchanged default
watch-first behavior). Android compilation, 16 JVM checks including end-to-end
counter isolation/bounding, and lint passed. Full Rust checks, rebuilt-ABI hosted
native runtime checks, release signing and physical Tab S9 measurements follow.
The two-second live target remains open until measured on the device.

### Visitor context follow-up

A closer review of [NewPipe's request helper](https://github.com/TeamNewPipe/NewPipeExtractor/blob/6faf40681eff7da6c7a45fb92e3eb633a60a062b/extractor/src/main/java/org/schabi/newpipe/extractor/services/youtube/YoutubeStreamHelper.java)
shows valid visitor context is required. ytui reuses its already-loaded, fresh
account/network-scoped browse visitor without waiting for feed locks. If missing,
the mobile extractor performs a small visitor_id bootstrap and retains it only
inside the pooled client for 30 minutes. Visitor data is included in both request
context and header; it is never copied into stream leases or timing reports.
An extra offline test verifies cold bootstrap is made once and subsequent videos
reuse it. A separate provider test covers busy locks, expiry and account isolation.
The timing report includes only an allowlisted direct/full/offline route label.
The fresh cloud probe with visitor bootstrap failed at the network transport
stage (169 ms); no visible-frame performance claim follows from this failure.

### First hosted runtime finding

Run 40864595426 executed 27 tests: 25 passed, one public test was explicitly
skipped, and the new startup test failed RGB corroboration after a sub-2000 ms
frame callback. This is not two-second acceptance. Codec surface release can
precede TextureView presentation. The view counter now waits a full UI frame
(two animation callbacks), scoped to the current request/generation. The benchmark
polls actual screenshots only within 2000 ms of the recorded tap and retains its
strict deadline, saving confirmation times and failure screenshots. Counters do
not replace screenshot proof. All prior native renderer/control regressions
passed on that run.

Background admission now waits before acquiring a per-video flight. This keeps
the final settled row eligible after older cancelled blocking work finishes,
while a tap on a waiting candidate can use foreground capacity independently.

### Second hosted run

Run 40867285845 passed startup playback and the new Performance/clipboard UI
check: view-frame counters were 1204 ms cold and 859 ms prepared, and RGB screenshot
requests started at 1749/902 ms. This is offline fixture evidence only. Capture
completion is now recorded immediately after screenshot acquisition (before PNG
encoding) and used as a conservative pixel-presentation upper bound, so a slow
screenshot request cannot create a false deadline pass.

The run had one existing account-navigation test failure: it queried Compose
semantics during the brief root-free interval after finishing AuthActivity.
Lifecycle logs show the app activity returning normally; the test now allows an
empty root set while waiting for the actual account screen, keeping its ten-second
deadline and final account-state assertions. No production account behavior was
changed. Full rerun remains required before publication.

The full multi-client fallback also used to discard per-client HTTP 429 errors
and continue, losing the service cooldown. It now returns the original rate-limit
error immediately. An additional offline TLS case verifies direct acquisition →
watch fallback → first-client 429 makes exactly three requests and never queries
another client. All successful CLI/download acquisition defaults remain unchanged.
