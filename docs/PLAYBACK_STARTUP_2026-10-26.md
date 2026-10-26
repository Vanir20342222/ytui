# Native playback and startup repair — 26 October 2026 (M4/M5/M8)

The reported Music item is the album recording of M83's “Midnight City”
(`K8v_DaCcORQ`), rather than the separate official music video. The installed
alpha.12 could report “Requested format is not available” and needed about
21.4 seconds to start this recording in an explicitly imported account session.

## Changes

- Update the native extractor to aagedal/yt-dlp-rs
  `93d561be519b2e46189ede4a8441a143892fd35b` (0.4.0-beta.1). Its authenticated
  client selection and SID request handling supersede the old anonymous client
  selection. Disable watch-history writes and dependency disk caching explicitly.
- Start individual playback with one compatible native client and omit optional
  initial-data/config requests. An eligible format/client incompatibility gets
  one wider native client pass with a fresh bootstrap. A known 429, bot/login,
  proxy or permission error does not start this pass. Downloads keep their
  separate backend/client policy. Music links resolve the same recording through
  the canonical watch URL, avoiding an unnecessary additional Music client.
- Select native formats before opening mpv and check a one-byte range using the
  allowed stream identity headers. Try at most three rejected 403/404/410
  candidates within eight seconds; an explicit format is never substituted.
  Rate limits and transport errors stop the candidate loop. This avoids repeatedly
  extracting an entire recording when a different already-returned format works.
- After a selection settles for 750 ms, prepare one likely item without starting
  a player. Otherwise prepare the deterministic next queue entry. Navigation
  starts at most one preparation every three seconds. Foreground playback adopts
  matching in-flight work. Stop, account/config changes and shutdown cancel and
  reap owned work. Offline/demo/live/collection/external-only cases do not prepare.
- Cache at most 16 resolved entries / 512 KiB in memory, for at most five minutes
  and never past a URL expiry minus 30 seconds. Canonical aliases share an entry;
  audio, quality, proxy, runtime and explicit format choices remain distinct.
  A rejected current stream invalidates its entry. Account/config replacements
  discard all old streams. Background 429s cause a 60-second preparation cooldown.
- Detect a supported later PATH runtime even if an earlier Node shim fails.
  Ask a working Node for `process.execPath` before passing it to the solver,
  whose private working directory can break a project-dependent shim. This keeps
  the selected engine/version and saved explicit setting. The opt-in solver runner
  resolves the engine too, so Cargo's changed working directory cannot break it.
- Preserve an earlier 429 diagnostic even if many later warnings culminate in a
  generic format error, so cooldown instructions cannot turn into immediate retries.

Playback still defaults to native Rust, with no automatic Python yt-dlp launch.
Explicit overrides and separate download fallback remain. No executable is
installed by extraction. Stream URLs, cookies and account context are not durable
cache entries or committed fixtures.

## Reproduce

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2
tools/test-native-solver.sh
cargo run -p ytui --example playback_startup --locked -- \
  --url 'https://music.youtube.com/watch?v=K8v_DaCcORQ' --audio --rounds 2
# Optional: simulate time spent looking at the selection before pressing Play:
cargo run -p ytui --example playback_startup --locked -- \
  --url 'https://music.youtube.com/watch?v=K8v_DaCcORQ' --audio \
  --prepare-seconds 12 --rounds 1 --complete
```

The example uses temporary state and actual mpv null outputs, disables external
extraction, and never imports browser cookies. `--session-file` accepts only an
explicitly supplied test-session copy. Report preparation time separately from
first-progress latency; cache replay is not a cold-start measurement. `--complete`
requires real EOF and a clock within two seconds of the reported duration.

## Measured evidence

| Final native playback case | Measured result |
| --- | --- |
| Exact album track, imported session, cold | First decoded progress 9,127 ms; native Rust |
| Same process, cached replay | First decoded progress 493 ms |
| Exact album track after 12 s preparation | First progress 876 ms after Play; preparation time is additional |
| Same prepared recording, complete | Real EOF at 243.56 / 243.98 s; elapsed after Play 244.88 s |
| Public 10 s corpus | 3/3 pass; jN video at 240p, Big Buck Bunny at 1080p, Music audio |
| Native authenticated audio download | One actual media file, fully decoded by FFmpeg |
| Exact album track, anonymous | YouTube requires sign-in; surfaced account guidance, no Python fallback |

The earlier installed-track cold run took 21,378 ms. These are individual local
observations rather than a latency percentile or a controlled network experiment.
A 12-second preparation wait cannot be counted as a sub-second cold extraction.

Verification: the complete workspace run passed 282 deterministic tests, with 30
opt-in tests ignored, before the final URL-normalization and Node-engine changes.
An updated service run passed 149 tests after URL normalization and engine
resolution, including all five runtime regressions for configured/unconfigured
shims, explicit choices, supported versions and cancellation. Strict workspace/all-target Clippy and formatting pass.
Six final actual-player/FFmpeg service checks pass, including HLS identity and
active-stream loopback reconnect. The earlier pin/probe/preload checkpoint passed
all twelve actual local-media checks. Actual Node saved-player and permission
vectors pass 2/2 using the resolved engine.

One incorrectly invoked MPRIS run failed because the real desktop already owned
the bus name; the required `dbus-run-session` rerun passes. Initial solver-vector
runs exposed a working-directory-sensitive mise shim; the corrected runner passes
with the real engine. Neither failure is counted as a successful initial run.

Earlier diagnostic runs established full native EOF for this exact album
recording (243.45 / 243.98 seconds), and prepared/replayed native progress for
three additional screenshot recordings: Lean On, House of Memories, and I Really
Want to Stay at Your House. Those runs preceded the final single-client and
canonical-URL changes and are supporting evidence, not final-code results.

Private local logs are under `~/Work/ytui-playback-speed-20261026/`. Browser tests
use explicit Brave Origin import into isolated temporary state, disconnect it
and remove the copy afterwards. No browser store, account preference, queue,
bookmark or downloaded file is changed. Proton VPN remains Disconnected.

## Limits

Cold playback still needs YouTube metadata and may take seconds. Preparation
moves that work earlier; it does not buffer an entire video or evade consent,
private-video restrictions, bot checks or service cooldowns. Anonymous Music
browsing/playlist consent and human audibility remain separate acceptance gates.
These recordings and the short public corpus do not establish universal playback
or 99% watching reliability. Full desktop/VPN-loss acceptance is not inferred
from null-output or loopback tests.
