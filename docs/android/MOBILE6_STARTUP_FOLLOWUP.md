# M9: device startup follow-up, mobile.6

The Galaxy Tab S9 report from mobile.5 (`7d5267a`) **fails** the live two-second
objective. Completed video attempts took 11249/14617 ms to visible output;
resolution accounted for 10407/12193 ms. Completed audio attempts took 3049/5011 ms.
Other attempts never recorded output. These results supersede any inference from
fast generated-media fixtures: live startup remains unqualified.

## Findings and changes

- Direct player acquisition still enumerated optional HLS/DASH manifests after
  receiving usable direct streams. Those network calls inherited the 12-second
  socket timeout, could delay the foreground tap and occupied extraction capacity
  during preloading. Direct selection now runs without manifest I/O first. Only
  when it cannot satisfy the requested selection does it parse necessary manifests
  from the same response, before full native acquisition. Quality/codec preference,
  account/expiry and throttling rules remain in force; CLI/download enumeration
  is unchanged.
  When a direct video is below the selected height cap (or Best is selected),
  manifests remain necessary to discover a better resolution. A regression
  preserves a 720p HLS option instead of prematurely accepting direct 360p.
- The single pooled extractor was discarded when switching audio/video modes.
  A bounded four-client LRU retains account/network/quality/mode isolation while
  reusing the HTTP connection pools when switching back. Account invalidation
  clears every pool entry.
- Foreground Kotlin resolution shared eight workers with background requests.
  It now has two reserved workers; a real JNI device regression saturates all
  eight background workers and resolves a loopback video while they remain held.
- Starting the foreground source cancels speculative prefix I/O while retaining
  completed cache spans and stream leases. Background reads no longer compete
  with its initial audio/video buffer.
- Completed stream leases are checked before waiting for an in-flight refresh.
  A ready lease cannot queue behind that lock. Cache reports distinguish `ready`,
  `joined` (waited for an active extraction) and `miss`. The previous boolean
  called a joined preload “prepared”, so the 10407 ms prepared sample does not
  establish a completed-cache lookup cost. Its underlying delay still counts.
- Reports retain the original tap-to-output total, and add fixed numeric stages:
  offline lookup, Kotlin worker queue, Rust engine, flight wait, extractor slot,
  blocking worker, HTTP client setup, extraction, visitor/player API calls,
  stream processing and decoder/readiness. Timings returned by a cache lookup
  describe the current request, not the request that originally filled it.
  Explicit resolver failures/cancellations remain labeled and cannot later be
  counted as successful startup. No URL, media ID, cookie or arbitrary diagnostic
  string is copied into this report.

## Verification and open gate

Eleven offline TLS tests cover successful video/audio without optional requests,
manifest-only HLS without a watch-page refetch, restricted/challenged fallback,
visitor reuse, quality preservation and rate limits. Core regressions cover ready
cache lock bypass, joined-request labeling and bounded mode-specific client reuse.
Android checks include report privacy, failure/cancellation accounting and native
foreground queue saturation. Final native/build/signing evidence follows in the
release notes; implementation alone does not satisfy live acceptance.

The cloud's YouTube CONNECT denial remains. The owner must rerun mobile.6 on the
Tab S9 and share Settings → Performance → Copy timing report, including Next,
new videos and slow/failed attempts. The two-second live goal remains open until
those actual device measurements meet it.
