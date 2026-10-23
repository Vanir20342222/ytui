# Target PC network and performance pass — alpha.10

The subsequent [alpha.11 single-video sweep](SINGLE_VIDEO_SWEEP.md) records further
repairs, complete playback/output checks and the latest installation. Results
below remain the historical alpha.10 network/performance evidence.

2026-10-22, M2–M6/M8. This record concerns the user's actual CachyOS LTS PC,
not the earlier cloud/GitHub network failures. Tools: Rust 1.99.1, mpv 0.41.0,
FFmpeg 9.0.1, yt-dlp 2026.08.19, Node 26.8.1, Deno 2.9.6. Native solving selects
Node; the unsupported Deno version was not changed. Tests use isolated state and
public content without browser cookies. VPN, consent and account settings stay
unchanged. Audio/video outputs are null for unattended playback.

## Reproduced defects and fixes

| Reproduction | Change and evidence |
| --- | --- |
| `browse details jNQXAC9IVRw` said unavailable although native playback worked. | Read JSON metadata embedded in the ordinary watch page first, checking the requested video identity and session. This needs one request; the player API remains a fallback. Never evaluate JavaScript or retain embedded stream URLs in a page cache. Offline regression failed before repair; actual details and pagination now pass. |
| `radio --country Germany` returned zero stations. | Send names as `country` and two-letter codes as `countrycode`. The regression failed before repair; both `Germany` and `de` now return 80 live directory entries. See the [Radio Browser API](https://docs.radio-browser.info/#advanced-station-search). |
| Music download saved a canonical YouTube URL over the submitted Music source. | Preserve the submitted source while updating metadata, so retry/history retain Music context. Both an offline reducer regression and the real audio-download check failed before repair and pass afterwards. |
| Three identical/concurrent searches made three API requests. | Cache successful pages and share an in-flight request. The regression now requires exactly one API request, separate Music/YouTube results, and a new request after Refresh. |

## Caching and limits

- YouTube/Music pages: process-local, 64 entries / 8 MiB serialized weight;
  60-second lifetime, or five minutes for details. Home remains live. Each Browser
  owns its cache; provider, request, session/account and transport instances stay
  isolated. Disconnect/account/proxy replacement creates a new provider.
- Radio pages: process-local, 32 entries / 4 MiB, 60 seconds, shared across provider
  clones. Existing saved-catalog outage recovery remains available. Refresh
  invalidates the selected request. Switching the disk-cache path starts a new
  memory cache so a hit cannot suppress publication to the new directory.
- Native stream resolutions: process-local, 16 entries / 512 KiB, two minutes.
  Keys distinguish source, audio/video, quality, proxy and tools. Replays reuse
  extraction while valid; cancellation preserves the valid result. A failed mpv
  open clears it before URL renewal. Settings/session changes replace the cache.
  Signed stream URLs are never persisted or logged by the cache.
- Cache errors are not retained. Cancelled request owners release their lock;
  Refresh prevents an earlier in-flight result from repopulating the cache.
  LRU entry/byte limits, expiry, oversized entries, errors, cancellation and
  invalidation have deterministic coverage.
- Existing thumbnail memory/disk caching, concurrent request sharing, bounded
  prefetch and playback IPC batching are retained and exercised.

## Observed performance

These measurements are from live service requests and immediate cache hits on
this PC. They do not predict every network or feed.

| Operation | First request | Cached |
| --- | --- | --- |
| YouTube search | 1.487 seconds | 100 calls in 3.323 ms, about 0.033 ms each |
| Radio jazz search | 513 ms | 0.053 ms |
| Radio Germany/name | 205 ms | 0.056 ms |
| Radio Germany/code | 365 ms | 0.055 ms |
| Radio classical tag | 305 ms | 0.066 ms |
| Thumbnail | Fetched and decoded from the actual CDN | 0.073 ms memory hit; a new cache instance also restores the disk image |

The native replay regression drives the actual player launch boundary with a
synthetic cached stream and an invalid runtime path: it must reach mpv without
extracting again, retain the stream after cancellation, then invalidate a failed
open. It is not a measured live native replay speed claim.

## Checks actually run

| Check | Result |
| --- | --- |
| Full `cargo test --workspace --locked` | 154 passed; 22 opt-in checks excluded from this offline run. Includes actual PTY popup/settings/resize/terminal restoration, all theme/layout fixtures, queue recovery, migration, synthetic auth, process cancellation and network failures. |
| Explicit actual-tool suite | 8/8 passed: FFmpeg MP3/M4A/Opus/FLAC conversion, cuts/chapters, actual two-file yt-dlp playlist, mpv HLS/split streams/headers/controls and CLI progress/error handling. |
| Real Node saved-player challenge and permission vectors | 2/2 passed. |
| Lua player-menu checks | Passed with installed Lua 5.4. |
| Public radio/YouTube/cache tests | 4/4 passed: cliamp's 15-entry catalog and track library; directory discovery/search/country/tag/pagination; actual station and library-track mpv progress; YouTube search/details/pagination and actual thumbnail CDN/memory/disk paths. |
| Public YouTube suite, auto | 8 passed, 1 failed: anonymous Music search requires browser consent. Video/audio/Music playback, public search/details, video/audio downloads and two distinct playlist outputs pass. |
| Public YouTube suite, explicit yt-dlp | Same 8 passed, 1 consent failure. |
| Explicit strict-native YouTube video download | Passed; ffprobe verified real output. |
| Native playback corpus | 3/3 public cases pass ten seconds of actual mpv progress, including video, youtu.be and Music/audio. A startup sample does not establish 99% reliability. |
| Formatting, strict all-target Clippy, shell syntax, optimized build | Passed. |

## Reproduce

`tools/test-network.sh` collects all check groups and continues after a failure.
It returns nonzero if any group fails, including a consent gate. Its constituent
commands above were run during this task; the entire new wrapper was syntax
checked, not rerun as another complete suite. It installs no tools and imports
no browser session. Public fixtures can change.

Focused live provider/cache checks:

```sh
cargo test -p ytui-services --locked --test live_network -- --ignored --test-threads=2 --nocapture
tools/test-youtube.sh auto
tools/test-youtube.sh yt-dlp
YTUI_TEST_BINARY=target/release/ytui YTUI_TEST_PLAY_SECONDS=10 tools/test-native-playback.sh
```

## Installation and remaining acceptance

Both local launchers are updated to the optimized alpha.10 build. Restart an
existing ytui process to load it. The previous alpha.9 development launchers and
maintenance documents are backed up under
`~/Work/caelestia-customization/ytui-alpha10-20261022/before/`, preserving absolute
paths below that directory. Restore the saved `home/vanir/.local/bin/ytui` and
`ytui-rs` for binary rollback. Existing user settings, queue/history and media
were not modified by the tests or installation.

Anonymous Music browsing redirects this network to `consent.youtube.com`.
The suite keeps this as a failure. Complete consent in the browser and explicitly
connect the chosen profile through F2 → Account to exercise signed-in browsing;
the agent did not accept consent or import a real profile during tests. Real
account Home/library/expiry journeys, visible Ghostty graphics, physical audio
devices, full-length listening/watching and population reliability remain open.
