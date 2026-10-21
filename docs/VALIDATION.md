# Validation record — 2026-10-21

Environment: Linux x86_64 cloud workspace, Rust 1.99.1. This is not the user's
CachyOS desktop. All automated application tests use synthetic/offline data.

## Completed

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`: **70 passed**, zero failures; one real-media test is explicitly ignored in this command.
- `NO_PROXY=127.0.0.1,localhost cargo test -p ytui-services --locked real_ffmpeg_ -- --ignored`: **1 passed** using generated media, real FFmpeg/ffprobe and a synthetic loopback SponsorBlock service. CI now runs this check separately.
- `cargo build --release --locked`: optimized Linux binary produced.
- `bash -n install-rust.sh`
- Installed the release binary as `ytui-rs` into an isolated temporary directory
  with install-rust.sh; the installed launcher reported the expected version.

Coverage includes:

- Native rate-cap timing against the actual Rust CLI, cancellation during pacing,
  no-clobber multi-output preflight, and retention of a completed staged source
  after failed tagging without publishing/marking a final output.
- Real native SponsorBlock marking, one-second removal (duration checked with
  ffprobe), two chapter files, retimed chapter endpoints, and unchanged original
  fixture bytes. The loopback API verifies that no Cookie header is sent.
- Local resume excludes preview/live positions, never starts at launch, passes a
  saved start time to fake mpv, and does not treat an early window close as EOF.
- Offline catalog skips missing/sidecar files and remote thumbnails; offline mode
  holds remote jobs and rejects remote playback/provider navigation.
- Watch baseline/new-item/duplicate behavior, failed-poll backoff, and stale-result
  rejection after a pause. Live playlist polling still needs target acceptance.
- Private state permissions, locked-instance-safe cleanup of synthetic stale
  credential exports/manifests, and a version-1 database backup before version-2
  upgrade. No real credential contents are read during these checks.
- Audio-mode destination switching preserves custom paths. HTTP 503 Service
  Unavailable is transient; postprocessing failures do not redownload automatically.
- Daily midnight-window boundaries, small global bandwidth caps, scheduled-state
  persistence, cancellation/requeue at closure, manual-pause preservation, stale
  completion rejection and explicit headless `--now` handling.
- Opt-in clipboard baseline/change and exact-host checks using synthetic text;
  stale/disabled results cannot open a download. No actual clipboard is read.
- Native subtitle sidecar transfer from loopback, missing-language errors and
  overwrite refusal. A separate real-FFmpeg embedding smoke check is recorded below;
  live YouTube caption formats still need target acceptance.
- Profile/keymap roundtrip, duplicate-key rejection, verbatim unknown legacy
  values/profile backups, migrated desktop toggles and bandwidth windows.
- Filtered/grouped history preserves real queue indices and playlist order;
  bulk ranges reject invalid bounds; import is atomic/idempotent and does not
  trust executable hook settings in an export.
- Literal subprocess environment variables, resolved metadata passed to hooks,
  output capture limits and headless error propagation.
- Explicit VPN actions against fake nmcli, escaped profile-name parsing and
  refusal to operate on non-VPN IDs. No actual VPN command is executed.
- Notification body escaping is tested; D-Bus delivery/sound remains a desktop
  acceptance gate. No real notification server is contacted by the suite.
- Deleted-job metadata cannot recreate a removed row; pause/retry rejects earlier
  generations and completion cannot be followed by stale phase updates.
- SQLite instance locking, paused-state preservation, interrupted-state recovery,
  and repeated migration without duplicate jobs.
- Native transfer through the actual Rust CLI and supervised helper against a
  local HTTP fixture, with yt-dlp configured to a nonexistent executable. The
  downloaded bytes and reported final file were checked; this proves the default
  path does not call Python. It does not prove general YouTube compatibility.
- Explicit fake yt-dlp fallback reports postprocessing failure rather than marking
  a job complete at 100%; existing output is preserved.
- Local conversion publishes the changed extension and preserves the input.
- Both subprocess pipes drain; cancellation is bounded; a descendant cannot keep
  inherited pipes open after its owning child exits.
- Browser Origin directory discovery without reading a cookie database, synthetic
  cookie-domain restrictions, redacted Debug, and `0600` Netscape roundtrip.
- Legacy/current YouTube card shapes, Music album/song parsing, and continuation
  handling using synthetic JSON. No real account fixtures are committed.
- Fake mpv IPC verifies preview pause/restoration; captured invocation verifies
  muted ten-second preview and no implicit yt-dlp invocation in native mode.
- Radio source parsing and disk catalog fallback during a synthetic outage.
- Caelestia/legacy Omarchy parsing, live atomic palette replacement, and retaining
  the last valid palette through malformed intermediate writes, using a relocated
  ytui state directory while preserving the separate XDG desktop palette root. This test caught
  and fixed a watch-channel read-lock deadlock during implementation.
- Unicode input/paste and focus isolation, plus rendering all tabs/forms at
  80×24, 120×40, 180×50 and the minimal 35×12 layout.
- Feed-picker remapping, navigation without playback, and local-only offline
  choices. Selection stays visible through scrolling/resize, loaded content and
  errors coexist, remapped footer hints stay current, and thumbnails do not draw
  through overlays. Download counts/progress and playback timing are checked in
  dark, light and terminal ANSI palettes, including long titles at 35×12.
- All 13 theme entries now render four workspaces and twelve dialogs across five
  sizes (1,040 combinations), plus ten queue phases per theme. Text/selection role
  contrast and ANSI chrome pass; Catppuccin Latte muted/status colors were fixed.
- Rendered mouse targets, wide feed-pane focus and compact resize behavior pass.
  Thumbnail PNG decode/cache/byte/dimension tests exposed unnecessary upscaling;
  relative M3U/PLS, HLS, redirect bounds and Radio Browser failover also pass.
- Fake mpv IPC additionally checks seek, volume, mute and pause. Repeat/shuffle
  checks cover nonrepeating advances. CLI init/import/doctor/offline guard tests
  exposed and fixed initialization at new/relative custom configuration paths.

A separate pseudo-terminal smoke check launched the actual demo binary, typed
`qvds` in the search input without quitting, switched tabs, resized
120×40 → 80×24 → 120×40, and exited with status 0. Terminal ICANON/ECHO flags
were restored. The follow-up PTY check also opened statistics, cycled history
filters, edited `q` in the keybinding form without quitting, scrolled to the final
help shortcut, and restored terminal flags on exit. The new
[history](screenshots/history.png) and [keybinding](screenshots/keybindings.png)
screenshots show those actual demo panels. These [screenshots](screenshots/youtube.png) are actual terminal
output rendered with a monospaced font; the catalog is explicitly fictional.
`NO_COLOR` was removed only from that child environment for color capture.

Another PTY check launched the actual `--offline` binary against a synthetic
version-2 state database. The local catalog, resume menu, watch panel, local
search and resize/clean-exit checks passed without account loading or online
provider requests. [Offline](screenshots/offline.png), [resume](screenshots/resume.png)
and [watch](screenshots/watches.png) captures show fictional fixture data; no media
was played by that screen check.

The UI-polish PTY run exercised the real demo binary at 120×40 → 80×24 → 35×12
→ 120×40, opened the feed picker, edited `qvds` without firing global actions,
switched all four tabs and applied the light theme. Exit status was zero and
ICANON/ECHO were restored. The current [YouTube](screenshots/youtube.png),
[Downloads](screenshots/downloads.png), [feed picker](screenshots/feeds.png),
[compact](screenshots/compact.png) and [light theme](screenshots/light.png) captures
show this actual terminal output with fictional content. No account/network
provider or media device was used. The offline catalog/resume/watch PTY check
also passed again with the new layout.

The latest reproducible capture is `uv run --no-project tools/capture_ui.py`:
**27 PNGs, each 2560×1440**, including all 13 themes, four workspaces, forms,
history, offline library, resume, watches and compact mode. It operates actual
demo/offline binaries at 212×58 → 80×24 → 35×12 → 212×58, waits for generated
thumbnail fixtures and checks terminal restoration. The artwork is generated in
Rust; these are rendered terminal captures, not screenshots of a real Ghostty
desktop. CI also runs `python3 tools/check_screenshots.py` without image libraries.
See [UI_THEMES.md](UI_THEMES.md) and [FEATURE_AUDIT.md](FEATURE_AUDIT.md) for the
theme matrix and the complete feature-by-feature coverage/acceptance inventory.

## Live checks and boundaries

- The application fetched the live cliamp catalog: **15 channels**, all currently
  exposing track-library URLs. The Omarchy track endpoint was also inspected.
  No stream was played on an audio device.
- Public YouTube browsing was attempted and failed at network/proxy connection
  setup. The earlier Python baseline audit received proxy HTTP 403 from YouTube.
  No attempt was made to bypass the environment's networking controls.
- `doctor` reported FFmpeg present; mpv, cliamp, optional yt-dlp and configured Deno
  were absent. Their process contracts were tested with fixtures, not real playback.
- Real FFmpeg smoke check: generated a 0.1-second WAV, converted it through the
  Rust CLI to a separate MP3, and confirmed the resulting format/duration with
  ffprobe. This exercised the real conversion tool without an audio device.
- Real native subtitle smoke check: generated a one-second MP4 and synthetic VTT,
  served an HTML5 video page over loopback, and downloaded it through the release
  CLI/native worker with yt-dlp configured to a nonexistent path. ffprobe confirmed
  video, audio and `mov_text` subtitle streams; the VTT sidecar was retained with
  its expected contents. This checks actual FFmpeg embedding without YouTube or
  an audio device; it does not prove live YouTube caption compatibility.
- No real browser profile, cookie store, Google account or desktop keyring was read
  in this cloud workspace. No VPN or desktop theme setting was changed.

## Outstanding target-machine acceptance

Before replacing the default Python installer/releasing the Rust edition as stable:

1. Import Brave Origin on CachyOS and verify correct YouTube account/channel,
   Home/subscriptions/Watch Later and Music library/search/continuations.
2. Exercise supported native YouTube downloads and explicit fallback on unsupported
   videos, playlists and persistent resume. Check long transfers and disk/path errors.
3. Verify mpv video/audio output, preview restoration, rapid switching, radio stream
   reconnection, and returning from the real cliamp program.
4. Check Ghostty Kitty images and cleanup, Caelestia live palette changes, SSH/tmux
   fallback, and separate real Omarchy/GNOME sessions.
5. Exercise daily scheduling/rate transitions during long transfers, Wayland
   clipboard detection/review, notifications/sound, and optional NetworkManager
   VPN actions on the target desktop.
6. Upgrade a copy of real Python state, review deferred fields in PARITY.md, and
   exercise the Python rollback path. Resolve the deferred legacy-feature scope
   before switching the default installer.

These are release gates, not completed tests. The branch is reviewable and runnable,
but this record does not claim that all upstream/API/platform bugs are fixed.
