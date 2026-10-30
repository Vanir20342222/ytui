# M9: device startup follow-up, mobile.6

**Subsequent physical result:** mobile.6 also failed on the Tab S9 with Best
quality: completed videos took 6772–11048 ms. [The mobile.7 follow-up](MOBILE7_STARTUP_FOLLOWUP.md)
records these failures and the remaining HLS traversal/preparation work. The
earlier request for a mobile.6 retest below is historical.

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

## Qualified artifact

[Download mobile.6 APK](https://github.com/Vanir20342222/ytui/releases/download/android-0.2.0-alpha.12-mobile.6/ytui-0.2.0-alpha.12-mobile.6.apk)
· [Public checksum/inspection](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.6).
Source `97e0b758dfb9e6f4b703a84a029953018d5c596d`, code 6, package
`dev.ytui.android.stable`, Android 8+, ARM64/x86-64. The permanent signing
certificate matches mobile.5; install over the stable app without uninstalling.

- [Exact-main Android](https://github.com/Vanir20342222/ytui/actions/runs/40946992360): 27 native cases pass, one public opt-in skip. Includes the real JNI eight-worker saturation test, cold/prepared video pixels, fullscreen/native controls, restarts, themes, input/account navigation and offline regressions. No app ANR/Rust panic markers.
- [Exact-main Rust](https://github.com/Vanir20342222/ytui/actions/runs/40946992378): 331 workspace + eleven extractor + fifteen media/solver tests pass, 31 explicit ignores; formatting/strict Clippy pass.
- [Exact-main secret scan](https://github.com/Vanir20342222/ytui/actions/runs/40946992269) passes. Signing credentials remain outside Git; the temporary finished-APK transfer is removed after public verification.
- Both release ABIs, 18 JVM tests and lint pass. APK inspection verifies stable signature, source/version, alignment and notices. [Publisher](https://github.com/Vanir20342222/ytui/actions/runs/40949351492) transfers only the signed APK and never receives the signing key.

Actual-window pixel confirmation is **1267 ms cold / 840 ms prepared**; native
view-frame events are 571/390 ms. Both pixel deadlines remain 2000 ms. These are
loopback generated-media tests, **not live YouTube acceptance**. The APK's public
bytes were independently downloaded and match the signed local artifact:

`b00f080a18bf105bf2f97db4486a9a9b418ba7aaf32277097593cd408236971b`

Size: 56,188,294 bytes. Public release notes include the same remaining live gate.
