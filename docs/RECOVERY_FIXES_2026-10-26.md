# Recovery fixes after local acceptance — 26 October 2026 (M2/M5/M6/M8)

Baseline: `codex/local-acceptance-20261026` at `ebbae44`, including the user's
unchanged installer-fixture repair and [local acceptance report](LOCAL_ACCEPTANCE_2026-10-26.md).
The current target PC is Omarchy + Ghostty + Brave Origin. CachyOS/Caelestia
remain historical compatibility targets. No target-PC configuration, VPN,
browser session or installed binary was changed by this cloud follow-up.

## Bugs fixed

| Failure | Repair and regression |
| --- | --- |
| Native playback treated HTTP 429 as a transient stream fault and could schedule another full extraction after one second. Live playback also retried it on its short reconnect schedule. | Surface the rate limit for cooldown instead of scheduling another playback attempt. Typed HTTP errors, FFmpeg errors and common service spellings share classification. An accompanying “no eligible media”/signature message cannot trigger immediate auto fallback through another backend. Ordinary unsupported-format fallback remains available under the existing configured policy. |
| A download with zero retry delay could immediately repeat a rate-limited worker request. | Job-level retries after a reported 429 wait at least 60 seconds, preserving the existing maximum delay, retry budget and cancellation. Normal transient errors retain their configured delay. Worker/CLI errors do not expose Retry-After headers, so this minimum is not a claim to know or clear the server's actual cooldown. Browser feeds still obey the HTTP header directly. |
| After three reconnects, a live stream ending with clean EOF could emit `Ended`, advancing a queue, or exit without a terminal error. | A live disconnect remains a disconnect after the bounded budget. Emit a visible error, never queue completion; explicit Stop/account handoff still cancels scheduled reconnects. |
| Reconnects discarded the explicit external-backend flag and restarted a selected yt-dlp YouTube live stream through native extraction. | Apply the per-playback override to the replacement launch, while leaving the configured default unchanged. A scripted extractor/player boundary test verifies external selection and preserved pause/volume/mute. |
| Pause, mute, relative volume and speed changes were ignored between attempts because no mpv IPC socket existed. | Cache those controls during reconnect backoff and use them for the replacement process. Absolute pause works too; non-finite deltas are ignored and numeric bounds remain enforced. Preview controls retain their separate existing behavior. |
| Details treated `isLiveContent` as proof of a currently live broadcast, although archived recordings retain that flag. | Use explicit current-live metadata and reject ended/upcoming flags. Loopback watch-page fixtures cover archived/unspecified/upcoming/current broadcasts. Ordinary feed LIVE badges retain their existing parser. |

Four initial recovery tests were run against the old source and all failed:
rate-limit retry, exhausted clean live EOF, controls during backoff, and explicit
backend retention. They pass after repair. Additional tests cover bounded
attempts, Stop/account cancellation, finite/clamped controls, rate-limit fallback
precedence, download cooldown/cancellation, and broadcast metadata.

Rate-limit guidance also distinguishes an unspecified/radio service from a
YouTube-specific message. This is recovery guidance, not a new account or network
mutation.

## Checks actually run in the cloud workspace

- **264 deterministic workspace tests pass**, with **30 opt-in checks excluded**.
  Ten ordinary regressions were added to the accepted 254-test baseline.
- Formatting, strict all-target workspace Clippy, workflow syntax and whitespace
  checks pass.
- **12 actual local-media checks pass**, including one new real mpv reconnect
  fixture. One private-session D-Bus MPRIS check passes separately.
- The new fixture serves synthetic audio over loopback HTTP. A short stream reaches
  EOF, then the replacement mpv opens a longer stream. During backoff the test
  changes volume, mute, speed and pause; it verifies the actual replacement
  arguments/state, unpauses and observes decoded progress without a queue-completion
  event. Outputs are null. It does not simulate a cable or VPN outage.
- The cooldown runtime test verifies that a zero-delay configuration makes no
  immediate second request and cancels promptly. Pure policy cases verify the
  60-second floor and existing upper bound without waiting a full minute.
- GitHub's Rust workflow includes the new real mpv fixture before the release build.

Reproduce with installed mpv, FFmpeg and yt-dlp:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2
# Only the new real-player case:
cargo test -p ytui-services --locked real_mpv_stream_reconnect -- --ignored
```

## Local evidence and remaining acceptance

The October 26 report remains the authority for the tested PC checkpoint
`c6cf246`: native video/audio/Music, authenticated two-file playlist decoding,
Brave Origin import/expiry, Ghostty rendering and the Omarchy palette passed.
Its **2/3 native corpus result**, HTTP 429 case, anonymous Music/playlist
restrictions and missing human-audio/active-VPN-reconnect confirmation stay
recorded as such. Those results are not relabeled as passing by these repairs.

After updating/reinstalling the new main source, rerun the rate-limited corpus
case after the service cooldown and qualify actual active-stream VPN loss/recovery
locally. Anonymous consent/bot-confirmation requirements cannot be removed by a
retry-policy repair; use the explicit browser import flow when appropriate.
No cookies, authorization headers, private feed fixtures or signed stream URLs
are included in the fixtures or this report. Native playback remains the default;
external extraction is still governed by the explicit playback/download policies.
