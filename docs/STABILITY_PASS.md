# Alpha.8 stability follow-up (M3/M5/M8)

This pass improves the repair branch without changing sessions, backend choices,
user files, VPN settings or the read-only account policy. It is an alpha release,
with target-device/live-service acceptance still open.

## Fixed behavior

- Closing an already closed preview no longer resumes a player the user paused
  afterwards. Failed preview setup and Stop also clear the pending resume action.
- Preview volume, mute, pause, position and speed controls no longer overwrite the
  main player's persisted preferences. Repeated preview speed changes use its own
  speed. Previews still start muted; their label does not claim they stay muted
  after the user explicitly unmutes them.
- A malformed/missing later output record cannot erase earlier real downloaded
  files from the job. Completed files are registered before validating the whole
  tool response or invoking hooks. Tool-reported single-video failures also retain published
  files. The job remains failed when any required step fails.
- Empty final files cannot mark a download complete. Per-file metadata and
  duplicate-hook protection from alpha.7 remain intact.
- Tool errors survive later warning noise. stderr lines are bounded while still
  fully drained, the final meaningful error is prioritized, and the retained tail
  is limited to roughly 68 KiB. Normal stdout JSON/progress handling is preserved.
- Permanent proxy authentication/CONNECT-403 and read-only filesystem failures
  do not enter automatic download retries. Temporary connection resets and 503s
  retain bounded retries.
- Player failures have enough space for recovery guidance. Bot confirmation,
  sign-in, proxy, runtime/tool, storage, access, rate-limit and extraction failures
  have distinct instructions. Small windows retain the actual remapped Actions
  key. Failed download details put the error before format/path details.
- `doctor --network` now exits unsuccessfully for failed or empty provider probes.
  It still prints each provider result. Ordinary `doctor` remains read-only and
  does not import a session.

The new output-retention/empty-file tests and an extended preview lifecycle test
**failed before the fixes**. The other additions cover error classification,
retry policy, preference isolation, stderr bounds, UI guidance and CLI exit status.

## Checks

- **139 default workspace tests pass**, 17 opt-in checks excluded. Nine additional
  default tests; existing lifecycle assertions are extended.
- New recovery UI checks cover **13 palettes × three sizes**, including remapped
  Actions/Settings keys. Existing UI/forms/PTY/theme tests remain in the suite.
- Three actual **mpv 0.40** checks pass with null outputs: player/menu/SponsorBlock,
  split/audio-only loopback handoff with preview controls, and actual CLI playback
  progress/error reporting. The synthetic extractor in the handoff test is not
  a successful YouTube extraction.
- Real FFmpeg/ffprobe checks pass for **MP3, M4A/AAC, Opus and FLAC**.
- A new **actual yt-dlp 2026.10.21 + FFmpeg + ffprobe** check downloads two HTML
  media entries served on loopback, converts both to MP3, verifies both tracks and
  durations, and checks both distinct paths/source in the saved SQLite job.
  This uses genuine extraction/tool arguments, not a fake metadata executable.
- Formatting, strict workspace/all-target Clippy, shell syntax, actionlint,
  optimized build and isolated installer checks pass. Both installed launchers
  report **0.2.0-alpha.8**.

Optional real-tool checks:

```sh
cargo test -p ytui-services --locked real_mpv_ -- --ignored
cargo test -p ytui --test downloads --locked real_ffmpeg_native_audio_ -- --ignored
cargo test -p ytui --test downloads --locked real_mpv_playback_check_ -- --ignored
cargo test -p ytui --test downloads --locked real_ytdlp_playlist_ -- --ignored
```

Use `YTUI_TEST_MPV` / `YTUI_TEST_YT_DLP` for nonstandard tool paths. The ordinary
Rust workflow installs real tools and runs the external playlist check separately.

## Open acceptance

No new successful YouTube CDN extraction/playback/download is claimed. The last
live run on the GitHub runner passed public YouTube and Music searches, while
seven other checks in each backend mode received YouTube's bot-confirmation
request. The workspace separately has a proxy CONNECT failure. Alpha.8 does not
remove service restrictions. See [live tests](LIVE_YOUTUBE_TESTS.md) and
[remaining work](REMAINING_WORK.md).

Physical audio/display, signed-in Brave Origin, Caelestia/Ghostty and Proton VPN
remain target-machine checks. No real cookies were imported by this test pass.
Cancelled workers retain completed files on disk; durable per-entry checkpointing
during an aborted long playlist is still partial. Late generation events stay
rejected rather than restoring a removed job.
