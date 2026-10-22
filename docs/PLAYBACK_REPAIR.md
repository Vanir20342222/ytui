# Playback and playlist repair — M3/M5/M8, alpha.7

Report: every YouTube video first shows fallback, then `Playback failed`.
User environment: mpv 0.41.0, yt-dlp 2026.08.19, CachyOS/Caelestia/Ghostty,
Brave Origin and Proton VPN. Tool versions are user-provided, not executed here.

## Findings and implementation

- `--no-terminal` suppresses mpv diagnostics, including extractor failures. mpv
  frequently logs errors on stdout, whereas the old failure path read stderr only.
  Reproduced using actual mpv 0.40: failed opening exited 2 with empty captured
  stdout/stderr under the old flags. This also hid native-stream 403 errors from
  automatic fallback detection.
- Old fallback delegated extraction to mpv's bundled Lua hook and its option-map
  translation, and forced a hardcoded Chrome user-agent over extractor headers.
  The user's exact server-side failure could not be identified from the old generic
  message. Do not claim a missing runtime, outdated tool or VPN block was proven.
- Both explicit external playback and automatic fallback now invoke supervised
  yt-dlp directly for JSON metadata, with no download, no playlist expansion,
  explicit format, cookies and proxy, bounded socket/retries/deadline and cancellation.
  No executables are installed. External yt-dlp uses its supported runtime from PATH;
  the pinned native Deno setting retains its separate meaning.
- Validate HTTP(S) stream URLs; choose video/audio correctly even if format order
  differs; pass split audio to mpv. Forward allowed extractor headers, reject control
  characters, preserve comma values, and keep authentication in the private cookie
  file. Disable mpv's extractor hook. Pause/resume/start/preview/player preferences
  stay attached to the mpv process specification.
- Enable warning/error logging into captured pipes, disable terminal input/colors,
  and sanitize stdout and stderr before displaying failures. Keep specific extraction
  failures distinct from mpv opening failures. Synchronous start failures clear loading.

## Checks and boundaries

- Deterministic tests exercise split/audio selection, header precedence/filtering,
  unsafe metadata, separate cookie/proxy/format arguments, specific/redacted errors,
  cancellation and player state/events.
- A separate real mpv/FFmpeg fixture generates video/audio, uses a synthetic extractor
  returning loopback streams with required UA/Referer/comma-containing headers, and
  verifies actual video+external-audio and audio-only decoding plus pause. It also
  checks real mpv stdout errors survive capture. This is a real player with synthetic
  extraction, not a successful live YouTube test.
- Existing real mpv controls/menu/quality/SponsorBlock/replay test also runs locally.
  Its seek assertion now polls within two seconds: mpv can acknowledge seek before
  its decoder updates time-pos. The required skip/replay behavior is unchanged.
- CI installs mpv and runs the loopback external-handoff regression. It remains
  excluded from default offline tests because it requires real mpv and FFmpeg.
- Public YouTube watch access was attempted through the inherited proxy: CONNECT
  tunnel failed with HTTP 403 (curl exit 56). No proxy/VPN bypass or real browser
  import was attempted. Signed-in extraction, real YouTube/CDN playback and displayed
  mpv/audio on the user's machine remain unverified.

Exact final results: [VALIDATION.md](VALIDATION.md). Update instructions:
[RUST.md](RUST.md). Current branch: `fix/playback-fallback`; version alpha.7.

## Playlist first-song defect

The user subsequently clarified that a playlist saves only its first song. Dedicated
`/playlist?list=…` links were offered the default single-download mode; per-entry
metadata could also replace the saved playlist URL with a song URL. A later retry
would then have no collection source. The pinned native YouTube extractor explicitly
rejects playlists and video URLs containing list=.

- Dedicated playlist links now set playlist intent in the form, CLI, queue creation
  and imports. Playlist indices also carry collection intent. Ordinary watch links
  with list= stay single-video by default; `--playlist` or the form opts into all.
  Dedicated playlist links require the playlist toggle rather than silently taking
  one entry. Pure RD playlist links retain their list ID; watch-page mixes still
  do not expand implicitly.
- Native YouTube playlist requests fail before network or output with a clear
  supported-backend message; auto uses the explicit yt-dlp fallback. Strict native
  remains strict. Explicit settings are never rewritten.
- Entry metadata cannot replace the queued collection identity. Completed files
  remain recorded if a later entry fails; failure is still reported. Dedupe includes
  playlist/range intent so single-video history does not block a full collection.
  Music-tab pasted links default to audio. Imports preserve playlist/range intent.
- Native HTTP 403/404 extraction failures can use one automatic fallback; local
  destination/publication errors remain errors and do not launch another backend.
- CLI integration fixtures cover three outputs for Music/video playlists, explicit
  watch-page collection mode versus single-video mode, a two-item subset, strict
  native rejection, retained source/partial outputs in saved state and one-shot
  fallback for a real loopback HTTP 403. The extractor in these cases is synthetic.
- A separate actual FFmpeg/ffprobe check downloads generated MP4 audio over loopback
  with native Rust, converts it to MP3, verifies codec/duration and keeps the source
  unchanged. This confirms the pipeline, not live YouTube extraction.

After updating, re-add the original playlist link if an old job already contains
only a song URL. That lost playlist identity cannot be reconstructed safely.
