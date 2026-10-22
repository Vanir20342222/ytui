# Single-video playback sweep — alpha.11

2026-10-22, M2/M4/M8. Scope: the same public prerecorded video,
[`jNQXAC9IVRw`](https://www.youtube.com/watch?v=jNQXAC9IVRw), about 19 seconds,
including its Music URL and watch URL with playlist context. Its available video
heights include 144p and 240p. Higher resolutions unavailable in this fixture are
not claimed as tested. Tests run on the actual CachyOS LTS PC with mpv 0.41.0,
FFmpeg 9.0.1, Node 26.8.1 and installed yt-dlp 2026.08.19.

## Reproduced failures and repairs

Each of these defects was reproduced before its fix; the regression or original
live case was run again after repair.

| Defect | Repair and evidence |
| --- | --- |
| A successful process exit before duration was known could complete the video and advance the queue. Closing near the end also looked like completion. | The bundled mpv Lua script reports an explicit EOF marker. Completion requires that marker and decoded progress; known-duration files must also reach the expected end. Tests distinguish quit, unknown duration, real EOF and truncation. Full live completion passes in native/auto/external modes. |
| Truncated EOF could silently stop a finite stream. | An EOF far before the known duration becomes an explicit error. Native/auto native streams clear cached URLs, renew once and resume. Actual truncated playback resumes at 2.8 seconds. [mpv documents that EOF can also result from broken/incomplete streams](https://mpv.io/manual/stable/#events), so EOF alone is insufficient. |
| A process remaining alive without IPC/decoded progress could buffer indefinitely. | Bound startup at 90 seconds and stalled decoded progress at 45 seconds. Reap the old process tree and use the existing bounded retry/error path. Manual pause and a main player suspended for preview remain allowed. A real deliberately hung child now reports failure at about 90.1 seconds; seeded-clock regressions cover startup, decoder stall and manual pause. |
| Fast native CDN failures could skip renewal because the backend watch sender had already closed. | Read its final value even after channel closure. A closed-sender regression and real extraction/owned-process failure injection verify one successful renewal and a two-launch maximum on repeated HTTP 403. |
| Replacing a running executable broke native worker launches with a `(deleted)` pathname. | Linux launches the retained running executable through `/proc/self/exe`. A deterministic actual-CLI regression removes its original directory entry during a gated runtime probe, then requires the worker to reach a rejecting loopback proxy. No real external request is made in this regression. |
| Explicit external previews failed with “Requested format is not available” while full playback worked. | Muted previews select video-only formats up to 360p before a combined-format fallback. They do not require an audio stream. A synthetic extractor regression and actual external preview/control run pass. |
| Immediate restart intermittently reported another instance after acknowledged shutdown. | Explicitly unlock after the database owner is joined. A shared-open-file-description regression reproduces the fork/exec inheritance pattern deterministically and now reopens successfully. The original full-suite failure also passes after repair. |

There are six repair areas: completion/truncation, liveness deadlines, backend
notification, executable replacement, external previews and state-lock release.

## Live player acceptance

The harness uses the actual `Player`, process supervisor, stream resolvers,
native worker implementation, mpv IPC and window-menu script. Native mode points
external yt-dlp at a nonexistent executable. Fault wrappers record only IPC paths,
preview/mute flags and numeric resume positions; no stream URLs or headers are
logged. All state/media are temporary; browser sessions and VPN stay unchanged.

**25 scenario checks pass after repairs**, executed in focused batches as fixes
were made: 16 native cases, four auto cases, three explicit external cases, one
deliberate hung-startup case, and one displayed/muted desktop completion case.

| Native case | Verified result |
| --- | --- |
| `completion` | Entire video reaches actual EOF and emits the correct completed-media event. |
| `controls-preview` | Real pause stops the clock; seek/speed/volume/mute reach mpv; muted preview preserves a manually paused player; natural ten-second preview completion resumes a previously playing main player. |
| `replay` | Stop/replay advances again using the same player cache. One observed run: 6.376 seconds cold, 0.874 seconds replay. This is a case measurement. |
| `quality-144`, `quality-240` | Actual decoded heights are 144 and 240, respectively. |
| `quality-switch` | Switching to 144p while paused retains pause and the position near five seconds. |
| `audio` | Actual audio decoder opens and playback advances. |
| `watch-playlist-context` | The watch URL still resolves the selected video as one video. |
| `missing-runtime`, `missing-mpv` | Specific error leaves neither active nor buffering; correcting the configuration allows playback. |
| `cdn-renew-once` | One injected HTTP 403 renews through native Rust, then plays. |
| `cdn-resume` | Actual playback interrupted after three seconds renews and relaunches at the saved position, observed at 2.8 seconds. |
| `truncated-eof` | A successful mpv exit after only three seconds is detected as truncation and resumes through renewal, observed at 2.8 seconds. |
| `cdn-retry-bounded` | Repeated HTTP 403 stops with a visible error after exactly two launches. |
| `clean-exit` | Exit zero before playback is an error, never a completed video. |
| `cancel-loading` | Cancellation acknowledges within three seconds; subsequent playback succeeds. |

Auto mode additionally passes complete playback, controls/previews, audio and
truncated-EOF renewal. Explicit yt-dlp passes complete playback, controls/previews
and audio. The deliberate no-IPC test uses external mode to observe one startup
deadline without a native retry; it passes after about 90.1 seconds.

The displayed native completion run used the single confirmed Wayland session,
opened **gpu-next** video and **PipeWire** audio, stayed muted, and reached the
video's actual end. No desktop/session configuration was edited. This checks
output initialization and completion; it does not establish audible speaker
quality or a human visual-quality assessment.

## CLI, decoding and regression checks

- Seven selected actual-CLI checks pass in each of native and external modes:
  details, video/audio/Music playback, video and Music audio downloads, and an
  additional explicitly forced-native download diagnostic. All use this video.
  Download checks inspect tracks/duration with ffprobe and fully decode completed
  video/audio outputs with FFmpeg `-xerror -err_detect explode` into a null sink.
- Eight actual-tool regressions pass: HLS and split-stream handoff/headers,
  controls/menu, MP3/M4A/Opus/FLAC conversion, chapters/cuts, loopback playlist
  handling and playback CLI failure/progress reporting.
- The final offline workspace run passes **158 tests**, with 22 opt-in tests
  excluded. Formatting, strict all-target Clippy, shell syntax, Lua checks and
  optimized build pass. Offline fixtures also cover request rejection, retry
  classification/cooldowns, malformed responses, disk/path failures, stale
  generations, cancellation, queue persistence and process ownership.

## Reproduce

```sh
tools/test-single-video.sh
```

This explicitly runs public requests and real media tools, continues through
failed groups, and returns nonzero for any failure. It never imports a browser
profile or changes consent/VPN. The new wrapper was syntax checked; its component
commands and scenarios were executed during this task rather than another full
wrapper rerun. Startup-deadline acceptance deliberately takes about 90 seconds.

Focused commands:

```sh
cargo run -p ytui --locked --example playback_sweep
cargo run -p ytui --locked --example playback_sweep -- --backend auto --cases completion,controls-preview,audio,truncated-eof
cargo run -p ytui --locked --example playback_sweep -- --backend yt-dlp --cases completion,controls-preview,audio
cargo run -p ytui --locked --example playback_sweep -- --backend yt-dlp --cases no-ipc
```

For a displayed **muted** completion run, inherit the actual graphical session's
environment and add `--cases completion --desktop`. This option is excluded from
the shell runner so an unattended rerun does not open a window.

## Limits and installation

This is acceptance of one public prerecorded video and defined failure cases.
It does not cover every YouTube video, high resolutions absent from this fixture,
live/DRM/age/private/account-restricted content, alternative language tracks or
all possible service/network changes. Anonymous Music browsing's consent gate
from the earlier report remains open; Music playback of this URL passes.

Both `~/.local/bin/ytui` and `ytui-rs` now report **0.2.0-alpha.11**, installed
through the repository `install.sh`. The installed doctor selects Node and
reports the existing native-playback/auto-download settings without opening a
browser store. Restart an existing ytui process to load the updated application.
An isolated post-install check using the installed launcher advanced this video
through native Rust to 10.066667 seconds (duration 18.933333).

The previous alpha.10 launchers and host maintenance documents are backed up at
`~/Work/caelestia-customization/ytui-alpha11-20261022/before/`, preserving absolute
paths below that directory. Restore the saved `home/vanir/.local/bin/ytui` and
`ytui-rs` to roll back the binary. User settings, queues/history, downloaded media,
sessions, VPN, packages and desktop configuration were not modified.
