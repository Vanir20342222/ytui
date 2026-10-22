# Validation record — 2026-10-21

Environment: Linux x86_64 cloud workspace, Rust 1.99.1. This is not the user's
CachyOS desktop. All automated application tests use synthetic/offline data.


## Alpha.7 playback/playlist repair evidence (M3/M5/M8)

User reports mpv 0.41.0 / yt-dlp 2026.08.19 and fallback followed by `Playback failed`,
plus playlists saving only the first song. Tool versions are user-provided.

- `cargo test --workspace --locked`: **119 passed**, 8 opt-in tests excluded.
  Includes all existing UI/theme/settings/actual-PTY tests and new explicit external
  extraction/headers/arguments/cancellation, visible mpv stdout failures, player
  error state/events, playlist source/intent/import/dedupe and real-executable
  multi-output CLI/state fixtures. Extractor responses in these cases are synthetic.
- Formatting and strict workspace/all-target Clippy pass. Optimized release build
  and actual isolated `install.sh` pass; both installed names report alpha.7.
- Separate real mpv 0.40 **two tests passed** with null outputs: generated local
  controls/menu/quality/SponsorBlock/replay and external-extraction handoff to
  header-requiring loopback split-video/audio and audio-only streams. A controlled
  missing-file error verifies stdout diagnostics. The seek assertion waits for the
  asynchronous decoder update within two seconds; it still requires actual skip.
  mpv and missing libraries were unpacked from official Debian packages outside
  the repository. The user's displayed mpv 0.41/device was not tested here.
- Separate real FFmpeg/ffprobe MP3 test **passed**: Rust downloads generated MP4
  audio over loopback, real FFmpeg converts it to MP3, ffprobe confirms audio/codec/
  duration, and source bytes stay unchanged. No yt-dlp executable was available to
  that test; successful live YouTube audio extraction is not claimed.
- A loopback HTTP 403 fails strict native and triggers exactly one external fallback
  in auto; an invalid local destination triggers none. Playlist CLI cases preserve
  all three outputs, a two-item subset, and partial outputs on later failure, while
  retaining the original URL in SQLite. Watch-page single-video behavior is checked.
- Public watch-page probe through the inherited proxy still **failed**: CONNECT
  tunnel HTTP 403, curl exit 56. No browser cookies, consent actions, VPN mutation or
  network bypass were used. No successful live YouTube/CDN playback is claimed.
- CI installs mpv/FFmpeg and runs the real external-handoff and MP3 regressions
  separately; real-service tests remain opt-in.

Reproduce optional local checks with installed tools:

```sh
YTUI_TEST_MPV=/usr/bin/mpv cargo test -p ytui-services --locked real_mpv_ -- --ignored
cargo test -p ytui --test downloads --locked real_ffmpeg_native_audio_ -- --ignored
```

Implementation and recovery of old first-song jobs: [PLAYBACK_REPAIR.md](PLAYBACK_REPAIR.md).

## Alpha.6 automatic feed recovery evidence (M4/M5)

- `cargo test --workspace --locked`: **106 passed**, 6 opt-in tests excluded.
  Includes 12 new recovery/provider/controller tests and the real-executable PTY
  navigation/popup/resize/terminal-restoration test. New recovery states retain
  content across all 13 themes at two sizes (26 states); existing theme/settings/
  download-form coverage also passes.
- Loopback requests exercise fresh embedded WEB/Music Home data, single-use and
  freshness, bare-shell recovery, one-shot stale-context renewal/TTL, explicit
  notices, transient/permanent/JSON/API errors, Retry-After seconds and HTTP date,
  and connection loss. No browser cookies or account credentials are used.
- Controller events exercise automatic tick retry of the failed continuation
  without a refresh key, cooldown/hidden/offline behavior, stale-generation
  rejection, usable existing cards, selected-media/history stability, overlapping
  pages/token cycles/empty-run limits, viewport prefetch and duplicate Next actions.
- `cargo fmt --all -- --check`, strict workspace/all-target Clippy and optimized
  `cargo build --release --locked` pass. Actual `install.sh` into an isolated
  directory passes; both installed `ytui` and `ytui-rs` report `0.2.0-alpha.6`.
- Current public **YouTube Home and Music Home CLI probes both failed** at the
  configured proxy tunnel (`cannot reach … tunnel error: unsuccessful`). No
  network bypass was attempted. These are failed live probes, not feed success.
  Signed-in Brave Origin/Proton/Ghostty acceptance remains open.

Implementation/limits: [FEED_RECOVERY_PLAN.md](FEED_RECOVERY_PLAN.md).
Historical evidence below applies to the earlier feature work; unrelated real
mpv/FFmpeg/SponsorBlock probes were not repeated for this feed-only change.

## Alpha.5 playback/navigation evidence

This supersedes alpha.4 UI/backend descriptions lower in this historical record.
The user confirmed automatic yt-dlp fallback for native extraction limitations.
Existing explicit backend settings are preserved; new configs recommend `auto`.

- 94 deterministic Cargo tests pass, plus the separate real FFmpeg fixture.
- 1,170 settings and 390 categorized-download states cover 13 themes / five sizes.
  Actual PTY interactions exercise Actions, forms, popups, Unicode, resize and
  terminal restoration. 50 WQHD captures are generated with Rust, using fictional
  data and a fictional active player strip.
- Regression checks cover one-shot auto fallback/strict native, sized/coalesced
  thumbnail memory/disk cache/restart, continuation deduplication/selection,
  station-vs-track selection, next-song queueing, player preference persistence,
  batched IPC IDs/timeouts, SponsorBlock filtering/overlap/replay and auth-free API.
- Bundled player menu syntax/navigation/quality/audio/SponsorBlock/close passes a
  Lua 5.4 test with a synthetic mp API. CI installs Lua and runs the same test.
- All four opt-in live YouTube tests were run again and **failed**: three at proxy
  tunnel connection, the native video transfer at its HTTP request. A separate
  public watch-page curl returned HTTP 403. No cookies or network bypass were used.
- Live Cliamp catalog and Lofi track-library HTTP requests succeeded. A real mpv
  stream probe decoded three seconds of Lofi (44.1 kHz stereo) through the configured
  proxy into null audio output; startup, time progression and clean EOF were checked.
  Actual device output, metadata refresh and reconnect remain open.
- Public SponsorBlock lookup for the yt-dlp test video returned HTTP 404 (no stored
  segments). Live marked-video skipping was not claimed; API parsing/replay are
  covered by loopback requests and real mpv synthetic marks.
- Real Ghostty image protocols, signed-in Brave Origin/keyring, Proton routing,
  graphical mpv window interaction and audio/video output remain target gates.
  Terminal moving video is not enabled. Browser categories are supported where
  the server supplies a recognized response; they are not hardcoded website clones.

A real mpv 0.40 headless regression passed separately using generated MKV media
with two audio streams and subtitles. It checks production startup/IPC, pause within
500 ms, volume/mute/speed, seek, audio/subtitle tracks/visibility, the bundled Lua
menu and quality re-resolution with pause/preferences preserved, SponsorBlock
skip/undo and source-byte preservation. The Debian binary and six runtime packages
were unpacked under /tmp; no system installation or browser/session was used.
Null audio/video outputs exercise real decoding/control without a display/device.

Reproduce with current mpv and FFmpeg installed:

```sh
YTUI_TEST_MPV=/usr/bin/mpv cargo test -p ytui-services --locked real_mpv_ -- --ignored
```

Reproduce the Lua check with `lua5.4 crates/ytui-services/tests/player_menu.lua`.
Locally it was executed through installed liblua5.4 with the same synthetic API;
no production Python dependency was added. Re-run failed live checks explicitly:
`cargo test -p ytui --test live_youtube --locked -- --ignored --nocapture`.

## Completed

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`: **94 passed**, zero failures; two media tests and four live-YouTube tests are explicitly ignored in this command.
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

The latest reproducible capture is `cargo run -p ytui --example capture_ui --locked -- --binary target/debug/ytui`:
**43 PNGs, each 2560×1440**, including all 13 themes, four workspaces, forms,
all eight settings categories, additional popups, history, offline library,
resume, watches and compact mode. It operates actual
demo/offline binaries at 212×58 → 80×24 → 35×12 → 212×58, waits for generated
thumbnail fixtures and checks terminal restoration. The artwork is generated in
Rust; these are rendered terminal captures, not screenshots of a real Ghostty
desktop. CI checks PNG headers and manifest completeness through the Rust
`repository` integration test; no repository Python scripts are needed.
See [UI_THEMES.md](UI_THEMES.md) and [FEATURE_AUDIT.md](FEATURE_AUDIT.md) for the
theme matrix and the complete feature-by-feature coverage/acceptance inventory.

## Live checks and boundaries

- Alpha.4 ran `cargo test -p ytui --test live_youtube -- --ignored --nocapture`.
  **Four tests failed**: public YouTube search, Music search and video details
  failed at proxy tunnel setup; native Rust download of the small public test
  video `BaW_jenozKc` failed on its HTTP request. The tests use isolated paths,
  no browser session, native backend and a temporary download directory.
  Signed-in feeds/playback remain unverified. Run these opt-in checks only when
  live access is available; default CI keeps them ignored and offline.
- Alpha.3 public probes used the actual optimized binary with isolated XDG
  paths, no browser session and the inherited environment proxy. The cliamp
  endpoint returned **HTTP 503**; both `doctor --network` and `radio` successfully
  returned **80 real Radio Browser stations**, clearly labelled as a fallback.
  No station stream was played. Directory discovery and a direct directory
  query also returned HTTP 200 in an independent anonymous endpoint check.
- Alpha.3 YouTube and Music public searches failed at proxy tunnel setup
  (`client error (Connect): tunnel error: unsuccessful`). Independent anonymous
  endpoint probes received tunnel HTTP 403. These failures prevent claiming
  live YouTube/Music acceptance. They do not establish how the user's Proton
  exit behaves. No VPN setting or proxy restriction was changed.
- The user's screenshot identifies their installed binary as **alpha.1**.
  PR #5 UI changes were merged into the PR #4 media branch, while PR #4 remained
  open against main. [Update instructions](RUST.md#updating-an-older-rust-alpha)
  explicitly select the provider-fix branch, reinstall, and verify alpha.3.
- An earlier application check fetched the live cliamp catalog: **15 channels**, all then
  exposing track-library URLs. The Omarchy track endpoint was also inspected.
  That earlier success is not evidence of current catalog availability.
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

## Alpha.3 provider regression coverage

Seven additional default tests exercise complete loopback requests and failure
handling: same-origin bootstrap redirects and loop limits; refusing external
origins before sending synthetic Cookie/Authorization headers; consent/sign-in
instructions without leaking redirect queries; separate WEB/WEB_REMIX bootstrap
caches, client versions and visitor headers; search and search continuations;
anonymous Home messages and sanitized upstream empty-feed text; HTTP 403/429;
public catalog redirects; cliamp HTTP 503/empty catalog/redirect-loop fallback;
and retaining connection causes without exposing URL query tokens.

The existing headless setup test also verifies `doctor` reports its executable
and refuses `--offline doctor --network`. Default CI does not contact providers.
Consent redirects to another origin remain explicit browser/import actions;
the app does not automatically accept consent or bypass service restrictions.

## Outstanding target-machine acceptance

### Popup and settings coverage

The real-executable Rust PTY test opens, checks and dismisses these surfaces:

| Surface | Trigger / check |
| --- | --- |
| Help | `:help`; Keyboard & commands title |
| Theme chooser | `:theme`; all themes additionally applied during capture |
| General settings | `,`; all nine sections, narrow layouts, Unicode input |
| Keybinding editor | `:keys`; key validation also covered by form tests |
| Browser/account form | `:auth`; opens/cancels without reading a profile in demo |
| Download options | `:add URL`; audio/video/playlist options have controller tests |
| Playback queue | `p`; queue operations have core/controller coverage |
| Resume | `:resume`; empty and synthetic saved-position render/capture |
| Collection watches | `:watches`; empty and synthetic configured-watch capture |
| History/statistics | `:stats`, `:downloads failed`; filtering and group indices tested |
| Groups | `:groups`; group expansion has controller coverage |
| Feed picker | `:feeds`, wide `b`; navigation/remapping has controller coverage |
| Text/diagnostics/update | `:log`, `:update`; real terminal title and dismissal |

Separate settings tests render first and last fields in all eight categories,
all 13 themes and five sizes (1,170 states). Every one of the 53 fields belongs
to exactly one category. Tests verify draft roundtrips, new defaults, choice and
toggle editing, invalid-save recovery, Esc/Cancel, mouse categories/fields/buttons
and actual asynchronous file persistence. Actual PTY resize covers 212×58,
80×24 and 35×12 and verifies terminal flags are restored on exit.

The installer integration test verifies build failure retains the old launcher,
both installed names report the current Rust version, the compatibility installer
works, and replacing a symlink never overwrites its legacy virtualenv target.

### Remaining release acceptance

The user explicitly authorized Rust replacing the repository app/installer after
the gap inventory. Before declaring the Rust edition stable:

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
6. Upgrade a copy of real legacy state, review deferred fields in REMAINING_WORK.md,
   and exercise compatible Rust-state recovery. Old source is retained in Git;
   the installed `ytui` and `ytui-rs` launchers now both run Rust.

These are release gates, not completed tests. The branch is reviewable and runnable,
but this record does not claim that all upstream/API/platform bugs are fixed.
