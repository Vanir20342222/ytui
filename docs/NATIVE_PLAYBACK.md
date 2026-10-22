# Native playback — alpha.9 (M3/M5/M8)

The native watching request exposed three concrete integration defects: the
extractor revision was outdated, installed challenge runtimes were not detected,
and every fragmented format was rejected even when its URL was a playable HLS
manifest. Native request headers were also discarded before mpv opened the stream.

## Changes

- Pin aagedal/yt-dlp-rs at `f67d96bdf98c5b821d0c9bf6175d328a0fa8ceab`.
- Detect installed Node.js 22+ or Deno 2.9.7. Explicit runtime paths are validated,
  not silently replaced. No runtime is downloaded. Version probes are bounded and
  cancellable. The pinned dependency solves saved YouTube JavaScript locally.
- Preserve playable manifest/base URLs, audio/video roles and allowed HTTP headers.
  mpv receives direct streams with its Python extraction hook disabled.
- Resolve watch links as a single video even when they contain playlist context.
- Separate `playback_backend` (default `native`) from `download_backend` (default
  `auto`). Settings → Playback and searchable Actions expose playback selection.
  An older config containing only download backend settings gets native playback;
  a saved explicit playback override is retained. `Y` remains a manual external
  action, clearly labelled. No automatic Python retry occurs in native mode.
- Renew a failed native CDN stream once for eligible connection/HTTP errors,
  keeping the last position, pause, volume and speed. Authentication, bot and proxy
  failures do not trigger the renewal. A second stream failure is surfaced.

## Reproduce

```sh
cargo build -p ytui --locked
tools/test-native-solver.sh
tools/test-native-playback.sh
# Optional: one public URL per line and a longer playback observation
YTUI_TEST_PLAY_SECONDS=30 tools/test-native-playback.sh /path/to/public-urls.txt
```

The playback runner isolates configuration/state, preserves inherited network
settings, imports no cookies, disables external yt-dlp, and observes actual mpv
progress with null video/audio outputs. Music URLs request audio. Every failed
case remains a failure; repeated identical input lines count once. A successful
startup check is not proof of watching a complete video or of audible/displayed
playback on the user's desktop.

## Evidence and remaining acceptance

- 147 default tests pass; 18 explicit actual-tool/live checks are excluded.
  Formatting and strict workspace/all-target Clippy pass.
- Full GitHub Rust CI [39781634440](https://github.com/Vanir20342222/ytui/actions/runs/39781634440)
  passes on code commit `a638d8289c44fb01760fd542b340a69348d55248`, including the
  default suite, actual Node vectors, native HLS, FFmpeg audio, external playlist
  checks and optimized build. Later documentation-only commits do not change the
  application used by that result.
- Optimized build and isolated `install.sh` pass. Both installed launchers report
  `0.2.0-alpha.9`; the installed offline doctor detects actual Node 24 and the new
  extractor revision, reports native playback separately from auto downloads,
  and confirms browser stores were not opened. Installed native playback also
  reproduces the same proxy rejection with contextual network guidance.
- Actual Node 24.19.0 passes the pinned dependency's two saved-player challenge
  and permission tests, including signature/n solving. These are offline vectors.
- Actual mpv 0.40 passes native HLS handoff with a server that requires the native
  User-Agent, plus existing split-stream/header and controls/menu/SponsorBlock
  checks. The HLS check requires segment requests and an advancing playback clock.
- Real FFmpeg/ffprobe checks pass for native MP3/M4A/Opus/FLAC transfers. A real
  external yt-dlp two-entry loopback playlist regression also passes; it is
  separate from native-only watching evidence.
- The managed-workspace public native run recorded **0/3 passed** at the HTTPS
  proxy tunnel. No external extractor was started. Public service results remain
  separate from the passing offline workflow; a local fixture does not clear this gate.
- Public native CI [39781157622](https://github.com/Vanir20342222/ytui/actions/runs/39781157622)
  built the real application and recorded **0/3 passed** with the native extractor's
  “All player responses are invalid. Your IP is likely being blocked by Youtube.”
  This is the extractor's diagnosis, not proof that every such response is an IP
  block. No Python fallback was enabled. The UI now gives account/network guidance
  for that response instead of suggesting a runtime change.

**The requested 99% watching success rate is not established.** It needs an
independent representative public-video corpus, complete/long sessions, supported
qualities, Music/audio/video, account states and the target desktop/network.
Even 299/299 independent representative successes would only put the one-sided
95% exact lower confidence bound above 99%; three startup cases cannot support
that claim. Login/live/DRM and alternate-language track discovery remain open.
