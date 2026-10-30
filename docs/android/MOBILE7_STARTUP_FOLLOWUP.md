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

Build/native/signing/publication evidence will be recorded after the checks run.
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
deadline. Fresh native success, including prepared-segment reuse, is required.

The second [PR native run](https://github.com/Vanir20342222/ytui/actions/runs/41029222182)
confirms `requirements=1 unmet=1` before the loopback scope and passes the real
downloads/exports. HLS passes at 718 ms cold / 689 ms prepared window pixels,
including first-segment cache reuse. It still fails the existing initial-video
case: the emulator logs 902 invalid zero color-buffer migrations and displays a
black TextureView despite continued decoding. The offline Goldfish/Ranchu debug
renderer now avoids placeholder/detached surfaces and requests Media3's surface
recreation workaround. Physical devices and release builds retain the standard
renderer. No pixel check or startup deadline is removed; another full native
qualification is required before publication.
