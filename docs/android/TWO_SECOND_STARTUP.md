# M9: two-second tap-to-visible-video target

Latest physical evidence: mobile.8 with **Best** on the Tab S9 passes all six
supplied visible-frame/usable-start attempts. Five cold usable starts are
**1115–1752 ms**, one prepared start **632 ms**; none fail or remain pending.
[Published mobile.8 and full physical results](MOBILE8_INDEXED_STARTUP.md) preserve
the earlier mobile.7 failures (three cold starts at 2827–4117 ms). This qualifies
the observed Best-quality video sample, not every network/service or broader M9
gate. The exact-main synthetic indexed check is usable at 914 ms, with actual
window pixels at 1585 ms.

Measure from the actual tap until the attached native view presents its first
decoded frame. Extraction, offline lookup, service initialization, source
preparation and decoding all count. A poster, playing flag or decoder-only
counter cannot satisfy this target. Require active playback too; `usable_start`
is the later of genuine visible output and actual playback start. Ignore queued
TextureView updates until the current item's renderer has produced a frame.
Report cold and prepared starts separately.
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

### Third hosted run: restart and capture costs

Run 40869934177 exposed a real restart failure: System UI issued a delayed Stop
to the new session, and the service was released about two seconds after starting.
New playback now publishes its buffering state to the session before notification
initialization, and a preparation notification is reused only when its platform
session token matches the current service. External Stop commands remain honored.
An actual notification/service regression exercises three consecutive sessions.

The stricter full-display readback also exceeded the deadline at 2053 ms despite
a 990 ms native view-frame event. The pixel probe now uses asynchronous PixelCopy
of the shown, focused native player's rectangle in the actual activity window,
including its overlays. It records copy completion before PNG encoding and keeps
the same 2000 ms deadline/RGB thresholds. This avoids charging unrelated full-screen
capture overhead to playback while still requiring actual composed video pixels;
a TextureView buffer, poster or playing flag cannot pass it. Full-screen RGB and
native-control regressions continue separately in the existing media tests.

Direct app playback registers the service session only after source preparation
and synchronous state publication, so Media3's first notification update cannot
be scheduled from the empty onCreate state. Controller-bound sessions retain
Media3's normal registration behavior.

### Fourth hosted run: inherited animation layer

Run 40873191482 passed the restart regression and all other native checks; the
prepared-start pixel check failed. Its cropped window image contained the feed
while a later full-screen image showed video over a translucent player/feed.
The native view could inherit an in-progress music fade because the root changed
AnimatedVisibility's parameters after receiving the new video snapshot. Removing
enter/exit parameters did not remove that existing graphics layer.

Video now lives in a separate direct composition branch with no AnimatedVisibility
ancestor. Music retains its transitions. A one-frame lag between the root's distinct
mode flow and the player snapshot shows a plain placeholder instead of creating a
native video view inside the music animation. This fixes actual compositing rather
than weakening the pixel assertion; the 2000 ms copy-completion deadline remains.

The direct route also compares the selected stream with the quality actually
offered in the response under the same height cap/codec preference. Unsolved
adaptive challenges must not silently turn offered 1080p into a 360p muxed fallback,
or audio-only selection into a video download. An offline TLS regression verifies
the full route restores offered 1080p. Faster starts do not change quality policy.

### Fifth hosted run: one notification owner

Run 40876838187 passed the strict startup test: actual-window pixel-copy completion
was 1507 ms cold and 656 ms prepared (view-frame events 519/390 ms). These are
local generated media results, not live YouTube qualification.

The separate media runtime regression still saw System UI remove notification 4
and send Stop to the new playing session. Media3 1.8.0 source confirms its default
notification manager removes notifications while its asynchronous internal controller
is not connected. The app's independently posted preparation notification therefore
raced with that manager even after source-first session registration. Playback now
has one notification owner: Media3 creates the initial notification and manages
foreground transitions when its controller connects. No external Stop is ignored.
The restart regression now alternates six audio/video sessions and verifies the
active notification belongs to the current session, in addition to continued
playback. Full native rerun remains required.

### Qualified performance APK

[mobile.5 qualification](STARTUP_VALIDATION_2026-10-29.md) records the successful exact-main native/Rust/security runs, signed/public APK verification and strict cold/prepared window-pixel times. The first exact-main attempt was contaminated by a Google SDK Setup ANR dialog; screenshots and failure evidence remain linked. The fresh same-source run retains all assertions and the 2000 ms deadline. Live Tab S9/YouTube acceptance remains open.

### Physical Tab S9 report: mobile.5 misses the live target

The owner's completed video attempts took 11249/14617 ms to visible output, with
10407/12193 ms spent before resolution returned. The live objective is not met.
[mobile.6 findings and changes](MOBILE6_STARTUP_FOLLOWUP.md) address optional
manifest I/O, pool churn, foreground admission and misleading joined-cache labels;
new device measurements are required after qualification/publication.
