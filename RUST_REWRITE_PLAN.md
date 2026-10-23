# ytui Rust rewrite plan

M2–M6/M8 alpha.12 networking follow-up fixes details cooldown bypass, redirected
radio playlists/deadlines, blocking cover proxy routing and stale requests after
network-setting changes. See [the network review](docs/NETWORK_RELIABILITY_2026-10-23.md)
for deterministic/actual-tool checks and the separate public-service results.
The second pass extends this to background watches, radio mirror selection,
provider-scoped disk fallback, cached discovery and redirect upgrade/downgrade
chains, with 217 deterministic passes and ten actual local media passes.

M5/M7/M8 alpha.12 comfort follow-up adds explicit reviewed LRCLIB lyrics, offline
LRC/text, timed-line following, initial German UI labels and bilingual Actions
search. Late diagnostic/cache results cannot reopen dismissed panels. These are
source/UI changes with their own checks; live account/desktop release gates and
remaining sentence-level localization are tracked in docs/AUDIT_IMPLEMENTATION.md.

M2–M8 alpha.12 audit implementation: account/settings operations now coordinate preparation
and durable writes; queue intent, image generations/retries, optional cache writes,
partial-output access, conversion quality and output identity have regression fixes.
Setup/capability and allowlisted support panels, thumbnail-cache controls and bounded
feed/history windows are added. See [the implementation tracker](docs/AUDIT_IMPLEMENTATION.md)
for executed checks, remaining feature work and desktop/live acceptance gates.

M2–M8 full-app audit on alpha.11 (`e2b01ea`):
[prioritized bug and feature report](docs/APP_AUDIT_2026-10-22.md).
Nine bug/release entries, separately identified risks and acceptance criteria are
recorded with executable isolated repros. This is analysis, not their implementation.
At that audit baseline the development branch was alpha.11 and main shipped alpha.5. The alpha.12 implementation is now tracked above.

M2/M8 repository maintenance: removed unused direct dependencies, consolidated
acceptance-runner reporting, expanded dense Rust blocks and aligned the example
configuration/current guides with alpha.11. [The documentation index](docs/README.md)
separates current guides from dated validation reports. Compilation and syntax
checks are recorded in docs/VALIDATION.md; no new live acceptance is claimed.

M5/M8 CI follow-up: diagnosed the screenshot's cancellation as a stalled Ubuntu
package installation, added whole-operation deadlines/official HTTPS mirror
recovery and seven offline CI-setup regression groups. The public native runner
passes 3/3 again on the target PC. A separate hosted-runner player-response
rejection remains a real failed live check; it is not hidden by the setup repair.
See [CI playback diagnosis](docs/CI_PLAYBACK_DIAGNOSIS.md).

Alpha.11 single-video M2/M4/M8 sweep: reproduced and repaired completion/truncated
EOF handling, indefinite player stalls, missed native backend notifications,
worker launches after executable replacement, unavailable external preview
formats and a shutdown instance-lock race. Full live playback reaches EOF,
including one muted gpu-next/PipeWire desktop run. See
[the sweep evidence and rerun commands](docs/SINGLE_VIDEO_SWEEP.md) for the fixed
public fixture, failure matrix and remaining coverage limits.

Alpha.10 local M2–M6/M8 follow-up: public details/country/Music-source defects are
fixed; bounded provider and native-resolution caches avoid repeated requests.
154 offline tests, eight actual-media checks, two Node checks, four live provider
checks and native playback/download checks pass. Auto/external public suites each
pass 8/9 with anonymous Music consent still failing. See
[the target-PC report](docs/NETWORK_PERFORMANCE.md); account/device/reliability
acceptance remains open.

Local M5/M8 follow-up, 2026-10-22: replaced the PC's stale alpha.7 launchers with
the completed alpha.9 development build at `7cc5a43`. One public video advanced
past ten seconds through native Rust and real mpv null outputs; the user requested
stopping heavy work while gaming, so the release build, solver suite and remaining
live cases were interrupted. See `docs/NATIVE_PLAYBACK.md` for evidence, rollback
and the remaining displayed/audio-device/reliability gates.

Historical alpha.9 status: PRs #5–#8 are merged into the PR #4 media branch. Alpha.6 feed recovery is on `improve/feed-recovery` (PR #9); alpha.7 playback/playlist repair and alpha.8 stability follow-up are on `fix/playback-fallback` (PR #10). Alpha.9 native playback follows on `fix/native-playback` (then-draft PR #11). PR #4 and target-machine stable-release gates were open. For the later alpha.11 state, use the follow-ups and current audit above.
Baseline: `main` at `4bdee78d463740ed959a3e69da7b8c314be992e6`.

Alpha.8 follow-up (M3/M5/M8): reproduced output-retention/empty-file and preview
state defects, then added regression checks and actionable failure UI.
See [docs/STABILITY_PASS.md](docs/STABILITY_PASS.md) for evidence and remaining gates.

Alpha.9 follow-up (M3/M5/M8): update the pinned Rust extractor, detect supported
installed JavaScript runtimes, preserve playable manifests and stream headers,
and separate native-default playback from download fallback. See
[docs/NATIVE_PLAYBACK.md](docs/NATIVE_PLAYBACK.md). Local checks do not establish
the requested 99% live watching success rate.

Publication: the original plan was pushed to `docs/rust-rewrite-plan` (PR #1).
Live-service validation follow-up (M3/M4/M5/M8): a dedicated public YouTube CI
workflow and an isolated local runner now exercise real playback and two-entry
playlist downloads. See [docs/LIVE_YOUTUBE_TESTS.md](docs/LIVE_YOUTUBE_TESTS.md)
for commands, assertions and environmental evidence; setup is not live success.
The first implementation was merged from `rewrite/rust` (PR #2); follow-up work lives
on `rewrite/media-library` (PR #4), including the merged `rewrite/ui-polish` PR #5;
the provider follow-up PR #6 is merged there, with the Rust-only follow-up on `rewrite/rust-only-settings`;
the parity work in PR #3 is merged. See docs/RUST.md, docs/PARITY.md and
docs/VALIDATION.md for actual behavior, evidence and incomplete acceptance gates.

## Goal and confirmed decisions

Replace the Python/Textual application with a dependable Rust terminal application
for browsing YouTube, downloading media, listening to YouTube Music, and radio.
The primary acceptance environment is **CachyOS + Caelestia + Ghostty**, with
**Brave Origin** as the user's browser. UI behavior and
downloads are the user's biggest current frustrations.

Confirmed by the user:

- Keep a terminal UI, inline thumbnails, and mpv video playback.
- Rust owns the application; mpv and FFmpeg are external tools. The user later
  selected **native Rust extraction by default**, using aagedal/yt-dlp-rs, with
  an **explicit yt-dlp fallback** initially. In the alpha.5 follow-up the user
  confirmed recommending an `auto` mode: Rust first, external yt-dlp for supported
  extractor limitations. Explicit strict-native and yt-dlp modes remain available.
  The later watching objective supersedes the shared default: playback now defaults
  to native, with no automatic Python invocation; downloads retain auto. Existing
  explicit playback overrides remain supported. A 99% claim requires live evidence.
- Provide Downloads, YouTube, YouTube Music, and Cliamp / Radio tabs.
- Support browser-cookie sign-in, a configurable video-preview shortcut,
  attractive themes, and automatic system-theme following.
- Publish this plan before implementation so other agents can review it.
- `v`: short muted mpv preview; `Enter`: full playback or open the selected
  collection; `Esc`: dismiss/back. All actions are remappable.
- Implement cliamp's radio catalog and Radio Browser discovery inside Rust;
  optionally launch the installed cliamp program. A full port of cliamp's many
  unrelated providers is outside this rewrite unless requested.

Default release scope:

- Linux is the first release target; explicitly document macOS/Windows support
  separately. Discover/select the Brave Origin profile during setup; do not assume
  it shares the standard Brave profile path or that a generic Brave loader works.

This is a full rewrite delivered in verifiable stages. A search box with empty
Music/Radio tabs is not feature completion. "All bugs fixed" is not a testable
promise: track reproduced defects, regression coverage, and remaining limitations.

## Original baseline audit

At the planning baseline, the app was a Python downloader with queue/history persistence, Textual
panels, conversion, settings/profiles, cookie options, and VPN helpers. Personalized
YouTube browsing, a dedicated Music tab, a radio tab, and video previews need new
implementations. Existing cookie options configure extraction; they do not by
themselves implement an authenticated YouTube home/library client.

### Findings to carry into regression coverage

| ID | Evidence at the baseline | Required Rust behavior |
| --- | --- | --- |
| B01 | Reproduced offline: remove an item while `QueueManager._resolve_metadata` is waiting; the late result calls `save_queue_item`, restoring the deleted row while the visible queue stays empty. | Removed jobs stay removed. Every result carries job ID and generation; reject stale results before any state/database mutation. |
| B02 | `QueueManager` creates metadata tasks without retaining their handles; `stop()` joins download workers and the queue loop, then closes the DB. Metadata tasks and timed-out executor work can outlive that shutdown path. Static lifecycle finding. | Track all work, cancel/join it, reap processes, then close persistence. No late writes after shutdown. |
| B03 | Importing `engine/downloader.py` globally assigns `subprocess.Popen = TrackedPopen`; process ownership is inferred from thread IDs. Static coupling finding. | Each job owns its child processes and their termination; no global process interception. |
| B04 | Download success returns `ydl.prepare_filename(info)`, which needs verification after conversion/remuxing changes the extension. Static correctness risk. | Use the final after-move output path; only mark completion when the resulting file exists and postprocessing succeeded. |
| B05 | Download failure requeues immediately, although `retry_delay_seconds` exists; yt-dlp also has its own retries. Static behavior finding. | One documented retry budget, cancellable backoff, and separate transient/auth/permanent failure classes. |
| B06 | `QueueDatabase.close()` attempts to close every thread-local SQLite connection from the caller thread and suppresses exceptions. Static resource-ownership risk. | One DB owner or a managed pool with explicit shutdown; test cross-thread work and restart recovery. |
| B07 | Some existing regression tests use live YouTube and panel tests can initiate background services. | Default CI is deterministic and offline; real service tests are separate and explicitly requested. |

Audit verification on 2026-10-21:

- Installed the baseline in an isolated virtualenv with isolated XDG directories.
- Ran 17 selected existing tests: **16 passed, 1 failed**. The failing test,
  `test_unavailable_video_error_formatting`, contacts live YouTube and received
  a proxy `403 Forbidden`, so this is **not evidence that the tested video is
  unavailable or that playback works**. No attempt was made to bypass the proxy.
- Reproduced B01 with synthetic metadata and threading events, without YouTube:
  `removed_item_reappears_in_database=True`, `visible_queue_length=0`.
- This was a targeted audit, not a complete UI, download, or live-account test.

## Product behavior

### Shared shell and navigation

- Four always-visible tabs: **Downloads | YouTube | Music | Cliamp / Radio**.
- Shared now-playing bar, visible focus, contextual shortcuts, search/command
  palette, consistent back navigation, and persistent per-tab selection/scroll.
- Arrow and vim navigation, optional mouse support, Unicode-aware clipping,
  paste handling, and help generated from the actual action/keybinding registry.
- Typing in search/settings never triggers global playback or destructive actions.
- Responsive grid on wide terminals; compact list/detail mode on small ones.
  Loading, empty, offline, unavailable, and expired-session states have distinct
  messages and retry/sign-in actions. Keep loaded content visible during refresh.
- Browser discovery, network requests, image decoding, downloads, and process
  waits never block rendering/input. Render on changes with bounded animation
  ticks; coalesce progress updates instead of flooding the event queue.
- Provide a deterministic demo mode with fictional content for UI review,
  screenshots, and terminal recordings, without exposing an account.

### Downloads

- Add a URL or selection from any browsing tab; choose audio/video, resolution,
  codec/container, destination, subtitles, and metadata before enqueueing.
- Playlist selection supports individual entries/ranges and a clear bulk action.
  Keep single-video URLs with playlist context distinct from explicit playlists.
- Persistent ordered queue with bounded concurrency; pause/resume, cancel,
  remove, retry, reorder, completed history, and open/play completed output.
- Show real phases: resolving, queued, downloading, merging, converting,
  completed, paused, cancelled, failed. Unknown sizes/ETAs stay unknown.
- Pause stops and reaps the job's process tree, preserves resumable partials,
  and resumes through yt-dlp continuation where supported. Cancellation/removal
  never deletes completed media or unrelated files. Partial cleanup is explicit.
- Use yt-dlp machine-readable progress and `after_move` output reporting.
  Drain both output streams, bound diagnostic buffers, tolerate partial/malformed
  lines, and never infer final success from progress reaching 100%.
- Cover lost connectivity, unavailable/private videos, missing FFmpeg, disk full,
  unwritable destinations, duplicate files, Unicode paths, and postprocess errors.
  Do not retry permanent failures indefinitely or silently lower chosen quality.

### YouTube browsing and watching

- Public search, video details, channels, playlists, related videos, and paginated
  results; signed-in Home/recommendations, subscriptions, Watch Later, liked videos,
  personal playlists, and history where the session/API supports them.
- Cards show thumbnails, title, creator, duration/live badge, and available metadata.
  Collection navigation and continuation tokens preserve context and avoid repeats.
- Select a video to watch in mpv, enqueue playback, download, copy URL, or open
  in the browser. Video details include description and chapters when available.
- A Rust provider handles authenticated web browsing. yt-dlp handles extraction
  and downloads; cookies plus `ytsearch` are not a substitute for personalized feeds.
- Start with read-only account browsing. Likes, subscriptions, playlist edits,
  comments, and automatic write-back of watch history require separate explicit
  product decisions; do not add hidden account mutations.

### Thumbnails and video previews

- Use terminal capability detection and a `ratatui-image` adapter for supported
  Kitty/Sixel/iTerm2-style protocols; provide half-block or text fallback and a
  user override for tmux/SSH/terminals with incorrect capability reporting.
- Async thumbnail fetch/decode/resize, bounded memory/disk cache and request
  concurrency, stale-selection protection, and cleanup on resize/tab switches.
  Validate image dimensions and sanitize control sequences in remote text.
- Confirmed preview: remappable `v` opens a muted, low-resolution, approximately
  10-second mpv preview of the selected video. It is explicitly started, cancellable,
  and never starts merely because the selection moves. The first release uses
  an mpv window; in-terminal moving video is a separate optional investigation.
- Only one preview exists at a time. Define and test full-playback interaction:
  pause existing playback during preview and restore its prior state on exit;
  promoting the preview to playback replaces the old item intentionally.
- Preview extraction has a timeout; missing mpv or unavailable media produces
  an actionable message and leaves navigation usable. Embedded thumbnails and
  video playback are separate capabilities.

### YouTube Music

- Dedicated Music home, categorized search (songs/albums/artists/playlists),
  artist/album detail, personal library, liked songs, and playlists.
- Audio playback through the shared mpv controller with queue editing,
  next/previous, seek, volume/mute, shuffle, repeat, and persistent queue.
- Use a dedicated Music API adapter, evaluating `ytmapi-rs` first against required
  endpoints. Reuse the selected browser session while keeping Music and YouTube
  request contexts, parsing, pagination, and account selection separate.
- Do not label generic YouTube audio search as YouTube Music integration.
  Add lyrics and track-based radio after core library/playback is reliable.

### Cliamp / Radio

- Native tab with separate sources for **cliamp channels**, **Radio Browser**,
  favorites, and custom HTTP(S) stations. Fetch cliamp channels dynamically from
  `https://radio.cliamp.stream/stations`; do not hardcode today's channel list.
- Preserve the difference between live streams and channels exposing track
  libraries: live streams play continuously; track collections can be queued
  and browsed when the upstream service exposes them.
- Radio Browser supports search, country/tag filtering, pagination, favorites,
  station metadata, and documented server discovery/failover. Cache catalogs and
  allow refresh; outages of one source do not break other tabs.
- Resolve bounded M3U/PLS playlists, reject unsupported URL schemes, show ICY
  now-playing metadata when available, reconnect with limits, and show buffering.
- Offer explicit import of cliamp custom stations/favorites into ytui-owned data;
  do not modify the user's cliamp config. Optional "Open in cliamp" suspends the
  TUI, stops conflicting playback, launches the installed binary, and restores
  terminal state on exit. It does not attempt to nest another TUI in a Ratatui pane.
- cliamp is a Go application, not a Rust UI library. Native radio functionality
  and a child-process launch action are the confirmed integration boundary.

## Browser sessions

- First-run account setup offers anonymous use, browser/profile selection, and
  Netscape cookie-file import. Import runs on the user's machine on request;
  do not request their cookies in chat or place them in the repository.
- Evaluate the Rust `rookie` crate for scoped cookie extraction, and yt-dlp's
  existing browser-cookie support for extraction/playback compatibility. Prioritize
  Brave Origin on CachyOS: verify its installed package/profile locations and
  keyring backend rather than assuming standard Brave discovery covers it.
- Test Firefox/Chromium-family profile selection; account for Zen/Librewolf/Brave,
  containers/custom profile paths, sandboxed installs, locked databases, and a
  locked or missing desktop keyring. Never require root or disable browser security.
- Retain only the domains needed by the YouTube/Google authentication flow,
  preserving domain/path/expiry/security attributes. Bind auth to allowlisted
  service origins; never send Google cookies to thumbnail CDNs or radio services.
- Validate a candidate session/account before replacing a working session.
  Support multiple accounts/brand channels explicitly if discovered; invalidate
  account-scoped caches and pending requests when switching or disconnecting.
- Prefer the OS keyring for retained credentials; an explicitly selected file
  fallback uses a private directory and atomic `0600` file creation on Linux.
  Child tools receive short-lived private cookie-file paths, not cookie values
  in arguments. Delete transient exports on completion and clean up stale ones.
- No cookie values, authorization headers, signed stream URLs, or sensitive
  query strings in logs/error reports. Disconnect removes ytui's stored session
  and account caches while leaving the browser profile intact.
- Session expiry, keyring failure, consent/bot challenges, and unsupported endpoints
  must be visible failures. Browser cookies do not guarantee access to every
  YouTube feature. Official Data API OAuth is not a replacement for Home/Music.

## Themes and automatic desktop integration

- Semantic palette roles: background/surface, primary/secondary/muted text,
  accent, focus, selection, border, success/warning/error. Avoid color-only status
  indicators and a required patched font. Provide reduced-motion settings.
- Curated themes: Catppuccin Mocha/Latte, Tokyo Night, Gruvbox dark/light, Nord,
  Rose Pine, and a restrained ytui dark/light pair. Review complete screens and
  selected/disabled/error states, not only color swatches. Preserve existing
  theme names through aliases or a documented migration map.
- Modes: explicit built-in/custom theme, follow system, and terminal ANSI palette.
  Allow an explicit system provider override; show the detected provider and
  fallback reason in settings/diagnostics.
- **Caelestia, first priority:** read `$XDG_STATE_HOME/caelestia/scheme.json`
  (default `~/.local/state/caelestia/scheme.json`). Upstream uses `mode` and a
  `colours` map. Map Material surface/onSurface/primary/error roles into ytui.
- **Omarchy:** read `$XDG_STATE_HOME/omarchy/current/theme/colors.toml`, with a
  legacy `$XDG_CONFIG_HOME/omarchy/current/theme/colors.toml` fallback. Support
  semantic/named colors and legacy ANSI keys, plus `mode`/legacy `light.mode`.
  Validate against fixtures from both generations rather than assuming one path.
- **GNOME and other desktops:** use `org.freedesktop.portal.Settings` appearance
  values and change signals first; use GNOME GSettings as an available fallback.
  Negotiate portal method/version support. Light/dark and accent hints are not
  an entire GTK theme palette: choose the matching curated light/dark theme and
  apply an available accent. This also covers compatible KDE/other portals.
- In automatic mode, use the active desktop's provider; absent that, use a
  validated Caelestia/Omarchy palette, portal, then terminal/default fallback.
  Stale files from an inactive desktop must not override a working active portal.
- Watch parent directories as well as palette files so atomic replacement and
  symlink changes work. Debounce updates, keep the last valid palette during
  partial writes, and fall back without crashing. Respect XDG paths everywhere.
- Read palette data only; do not execute theme scripts or alter the desktop theme.
  Generic custom palette files cover other dotfile setups without shell hooks.

## Rust design

Start with a Cargo workspace and three crates, splitting further only when useful:

| Crate | Responsibilities |
| --- | --- |
| `ytui-core` | Domain IDs/models, action/event reducer, download state machine, playback queue, configuration and migration contracts. No terminal dependency. |
| `ytui-services` | YouTube/Music/radio adapters, browser sessions, yt-dlp/FFmpeg process control, mpv JSON IPC, SQLite storage, image cache, desktop-theme providers. |
| `ytui` | Ratatui/Crossterm binary, input/focus/navigation, layouts, settings/help, CLI and diagnostics. |

Candidate libraries: Tokio, Ratatui/Crossterm, serde/TOML/JSON, reqwest, rusqlite
with a dedicated DB worker, tracing, clap, directories, notify, zbus/ashpd,
ratatui-image, keyring/rookie, and ytmapi-rs. Verify APIs, Rust version, dependency
features, licenses, and CachyOS build requirements during M0; pin dependencies
and commit `Cargo.lock`. These are candidates, not untested compatibility claims.

Core rules:

1. A single reducer owns visible application state. Workers send typed results
   through bounded channels; progress is coalesced. No service reaches into widgets.
2. Results carry request/job generation and account identity. Navigation changes,
   retry, cancel, remove, and account switching invalidate stale work.
3. Download state transitions are explicit and persisted transactionally. A task
   cannot complete, requeue, or restore a removed job. Playback and download queues
   are separate even when referring to the same media.
4. A process supervisor owns each child/process group, closes stdin, drains pipes,
   enforces deadlines, terminates gracefully then kills if needed, and always waits.
   Preserve required proxy/CA configuration and pass argument arrays without a shell.
   Use explicit yt-dlp options/config isolation so unrelated global configs cannot
   silently change output paths or invoke user hooks.
5. Use supervised mpv JSON IPC on a private Unix socket for full playback and
   previews. Pass canonical media URLs or resolved streams through a defined
   resolver boundary; refresh expired URLs. Raw cookie material never goes in IPC.
6. DB migrations are versioned, backed up, and tested. Enforce single-writer
   ownership; prevent a second application instance from racing the same queue.
7. Public metadata, account-specific feeds, cookies, and thumbnail caches have
   distinct storage lifetimes. Cache identity includes account/provider context.
8. `ytui doctor` reports dependencies, supported terminal image mode, desktop-theme
   provider, and sanitized auth status. Missing tools disable affected actions
   with clear guidance instead of crashing the entire app.

## Migration and compatibility

- The original staged migration kept Python runnable. On 2026-10-21 the user
  explicitly requested removal after documenting remaining gaps. That instruction
  supersedes the former installer/source-retention gate. The gap inventory was
  committed before removal; stable account/desktop acceptance is still open.
- Inventory every setting/command and label it retained, replaced, or deferred.
  Cover quality/container overrides, playlists, conversion, subtitles, metadata,
  SponsorBlock, profiles, proxy/bandwidth/schedules, duplicates, notifications,
  optional clipboard watching, export/history/stats, and post-download hooks.
- Back up and import existing `settings.json`, `keybindings.json`, profile JSON,
  and `queue.db` into versioned Rust config/state. Preserve queue order, paused
  intent, groups, paths, and history. Dry-run/report unknown fields; repeated
  migration must not duplicate jobs. Never overwrite the only copy of old state.
- Keep user hooks opt-in and run executable paths with argument arrays. Clipboard
  watching stays opt-in. Preserve proxy configuration. VPN/Proton/WireGuard helpers
  need an explicit parity decision: do not silently drop them or automatically
  change network connections during tests/startup.
- Mark every deferred legacy feature in release notes and obtain a scope decision
  before replacing the default installer. A smaller first milestone is not the
  finished rewrite.
- At the user's request, replace the installer/package metadata with Rust. Document Rust
  build/install, external tools (including any current yt-dlp JS runtime requirement),
  Arch/CachyOS packaging, upgrades, and a rollback path to the Python baseline.

## Implementation milestones and review gates

The module implementations and offline checks now exist. The status ledger below
distinguishes them from the live/target acceptance gates, which have not all passed.
Dependencies remain deliberate; no task is done merely because it compiles.

| ID | Work / deliverable | Depends on | Acceptance gate |
| --- | --- | --- | --- |
| M0 | Audit/legacy parity inventory, dependency/license decisions, fixture formats, Brave Origin profile discovery and Ghostty image support, provider/auth/playback spikes. | Plan | Demonstrate public metadata, scoped session validation, Music library access, mpv control, Caelestia palette parsing and cliamp catalog shape; record live checks blocked by environment. |
| M1 | Workspace, typed state/events/services, responsive four-tab shell, config/keybinding registry, demo mode, CI. | M0 | Navigate every tab and modal at 80x24/120x40; typing/paste/resize/quit work; no fake feature-complete tabs. Establish task/generation contracts and terminal restoration. |
| M2 | Supervised yt-dlp/FFmpeg, queue/persistence, retries, conversion, migration. | M1 | URL → successful verified final file; pause/resume/remove/retry/restart and B01–B06 regressions pass, including removal during metadata/postprocess. |
| M3 | YouTube browse provider, cookie/profile setup, personalized feeds, pagination, thumbnail cache/rendering. | M1; M0 provider spikes | Public and selected-account journeys work; correct account shown; expire/disconnect/switch safe; unsupported features reported honestly; no stale results or image residue. |
| M4 | Shared mpv playback, preview shortcut and controls, persistent playback queue. | M1, M3 | Selected video plays; preview is muted/bounded/cancellable; restoring prior audio and rapid preview replacement work; no orphan process/socket after quit. |
| M5 | Music home/search/library/artist/album/playlist UI and shared audio playback. | M3, M4 | Browse the user's Music library and play/queue tracks; shuffle/repeat/seek work; pagination/expired sessions covered. |
| M6 | Cliamp channels, Radio Browser discovery, favorites/custom station import and optional cliamp launch. | M1, M4; M0 catalog spike | Live and library-backed cliamp channels work where supported; station search/favorites persist; broken streams/reconnect/catalog outages and return from cliamp are handled. |
| M7 | Curated themes, live Caelestia/Omarchy/portal following, accessibility and UI polish. | M1; may overlap M2–M6 | Change Caelestia theme while app runs; switch light/dark; malformed/atomic file changes and portal absence recover; all tabs readable. |
| M8 | Full parity review, package/install migration, CachyOS acceptance, release docs/artifacts. | M2–M7 | All required journeys and mandatory checks pass; deferred features explicitly resolved; clean install + old-data upgrade + rollback exercised. |

The implemented vertical slice transfers media through the native Rust worker and
verifies final files. All four tabs call real provider adapters. Personal feeds
and desktop/audio behavior still need the target acceptance checks below; the
fixture/demo catalog is not counted as live-service validation.


### Implementation ledger (2026-10-21)

| Milestone | Current evidence | Status |
| --- | --- | --- |
| M0 | Legacy audit, license/API inspection, native transfer fixture, actual cliamp catalog, scoped auth and palette parsing. Evaluated ytmapi-rs; kept one isolated transport for WEB/WEB_REMIX. | Implemented spikes; YouTube/browser/device checks blocked in cloud. |
| M1 | Three-crate workspace, all four provider tabs, forms, Unicode input, remapping, demo, CI; render tests and PTY resize/restore check. | Implemented and checked offline. |
| M2 | Supervised native/explicit-fallback workers, queue/retry/cancel/reorder/persistence, final paths, local conversion, backups/import/export. | Core implemented; native persistent resume unsupported upstream and full legacy parity remains open. |
| M3 | WEB feeds/search/collections/details, scoped rookie import/validation/storage, account invalidation, async thumbnails. | Implemented; live account/Origin/Ghostty checks pending. |
| M4 | Native stream resolution, explicit fallback action, mpv IPC, queue and muted preview with pause restoration. | Implemented; fake IPC/process checks pass; real playback pending. |
| M5 | Dedicated Music context, categorized search, libraries and collection navigation, shared audio queue. | Implemented; live Music account tests pending. |
| M6 | Live cliamp catalog/track libraries, Radio Browser search/filter/failover/cache, favorites/custom import, external cliamp launch. | Implemented; actual audio/cliamp checks pending. |
| M7 | Curated themes, ANSI mode, Caelestia/Omarchy paths, portal signals/GNOME fallback, last-valid recovery. | Implemented; live file fixture passes; real desktop/graphics checks pending. |
| M8 | Default Rust installer with both launcher names, migration/gap inventory, 94 default tests plus real-media fixture, opt-in live probes and 50 native WQHD captures. | Repository transition authorized/completed; stable desktop/account release gate remains open. |

Backend decision: the native library is pinned to revision
`10223c019ef6ec5552627b90251d8e967404e440`. It is not a drop-in replacement for
Python yt-dlp: native YouTube coverage, playlist support and cross-process resume
are limited. Native subtitles, pacing, SponsorBlock and chapter adapters now exist;
unsupported codec/caption combinations need the fallback. The user confirmed
automatic fallback for supported native limitations in this follow-up; strict
native mode remains available. Do not present unverified live behavior as completed.
See [the usage guide](docs/RUST.md) and [remaining parity](docs/PARITY.md).

### Parity follow-up (M1/M2/M6/M7/M8)

- Added daily local-time download/rate windows, persistent delayed jobs and a
  conservative byte-based cap. Window closure invalidates/cancels active work;
  manual pauses remain intact. CLI window bypass is explicit (`--now`).
- Added opt-in Wayland/X11 clipboard detection with review, native desktop
  notifications/sound hints, profile/key editors, searchable history/statistics,
  group expansion, loaded-page bulk ranges, and safe JSON/text queue import.
- Native subtitle sidecars/embedding use pinned upstream APIs, isolated staging
  and no-clobber publication. Missing/unsupported features remain explicit errors.
- Migration preserves unmapped JSON/keymaps/profile backups and maps desktop and
  bandwidth toggles. Hooks receive resolved metadata via legacy YTUI_* variables.
- Added radio country pins and explicit NetworkManager VPN/WireGuard actions.
  Tests use fake nmcli only; standalone Proton/raw wg-quick parity is still open.
- Fixed small global caps rounding to unlimited, headless worker exit handling,
  bounded single-line process output, ANSI download-state colors, and desktop
  palette discovery when app state is relocated with `--state-dir`.
- Offline checks and target-machine limitations are recorded in VALIDATION.md.
  These changes do not close live account/device or stable packaging gates.

### Media/reliability follow-up (M1/M2/M3/M4/M5/M8)

- Native media/caption pacing with cancellable worker ownership; native chapter
  metadata/splitting and SponsorBlock mark/removal. Real FFmpeg/ffprobe fixtures
  verify markers, reduced duration, chapter outputs and unchanged originals.
- Native outputs stage privately and publish without clobbering existing files.
  A later postprocessor failure reports a recoverable source directory. Fixed
  HTTP 503 retry classification and audio-toggle destination selection.
- Explicit local resume on `u` / `:resume`, bounded private positions, offline
  catalog and startup mode, and early-window-close handling. The user selected
  local resume actions, no automatic playback or YouTube history write-back.
- Recurring collection watches with a complete initial baseline, duplicate checks,
  persistence, resource limits/backoff and account/generation invalidation.
- State format 2 adds a SQLite version-1 backup before upgrade; stale owned
  credential exports/manifests are cleaned only while holding the instance lock.
- Added sanitized diagnostics/update instructions and CI's explicit real-media
  check. Actual account/Origin/Ghostty/audio tests and stable packaging gates remain
  open; standalone Proton/raw wg-quick parity remains a separate scope item.

### UI polish and feature audit follow-up (M1/M2/M6/M7/M8)

- Compact tab/navigation shell, denser catalog rows, selected-item actions and
  adaptive thumbnail/details panels. Removed the empty thumbnail placeholder.
- Added a remappable `b` feed/library picker with tab-specific and offline-only
  choices. Footer/detail hints reflect the actual keybinding configuration.
- Download summary counts now distinguish queued, paused, failed and completed
  jobs; wide layouts show destination, errors, progress and ETA separately.
- Responsive player with visible timing, compact empty dialogs, explicit empty
  states, settings toggle indicators/field position, and a faded modal backdrop.
- Regression checks caught and fixed disappearing selected rows in 35×12
  windows. Offline render checks cover selection/scrolling, remapped hints,
  thumbnail overlay bounds, status counts and playback time in dark/light/ANSI.
- PTY checks exercise the real binary at 120×40, 80×24 and 35×12, feed selection,
  input isolation and terminal restoration; screenshots are fictional demo data.
  Real Ghostty image-protocol/device acceptance remains open.

- The user requested Lazygit/Lazydocker-style panes and WQHD captures. Wide
  layouts now have status/workspace/feed panes and a larger detail/thumbnail area;
  `b` focuses the feed pane. Replaced hard-coded mouse coordinates with rendered
  targets for workspaces, feeds and scrolled lists. Narrow layouts stay usable.
- All 13 theme entries render 1,040 tab/dialog/size combinations, plus all ten
  download phases. Added contrast checks and improved Catppuccin Latte's muted,
  warning and success roles. Every committed PNG is 2560×1440; CI checks headers.
- Rust-generated demo artwork and a reproducible PTY capture helper exercise the
  actual image renderer without provider access. Demo image generation is off
  the input thread. Text/selection/image geometry and ANSI colors are checked.
- The feature audit added thumbnail decode/cache/limit checks, radio wrapper/HLS/
  failover tests, playback-control IPC and shuffle/repeat coverage, command/form
  checks, and CLI setup/import/offline guards. It found/fixed unnecessary thumbnail
  upscaling, relative M3U handling and new/relative config initialization errors.
- Full evidence and explicitly unverified live behaviors are in
  docs/FEATURE_AUDIT.md; docs/UI_THEMES.md links the 27 WQHD captures. Real account,
  Origin/keyring, Ghostty, media devices and stable packaging remain release gates.

### Provider failure follow-up (M1/M3/M5/M6/M8)

- User screenshots reproduced alpha.1's hidden radio connection error, Music
  bootstrap HTTP 302 and empty anonymous Home. The user's installed alpha.1
  predates the UI branch; PR #4 remains unmerged into main. Reinstall/version
  instructions now identify the exact branch and executable; version is alpha.3.
- Added bounded same-origin bootstrap redirects without forwarding browser
  credentials to another origin. Consent/sign-in redirects give browser/import
  instructions; no consent choice is made automatically. Public service
  redirects may change host but never downgrade HTTPS.
- Empty anonymous Home explains search/sign-in, provider empty-feed messages
  are preserved, and the central empty/error pane shows the complete explanation.
- Radio catalogs follow public redirects; server requests run concurrently with
  eight-second deadlines and retain sanitized failure causes. A saved cliamp
  catalog is preferred during an outage; otherwise a clearly labelled Radio
  Browser fallback keeps station browsing available.
- Added opt-in `doctor --network` public probes without browser-cookie access,
  cached success masking, VPN changes or network activity in ordinary doctor.
- Request-level loopback coverage now checks redirects, origin scoping, separate
  WEB/WEB_REMIX contexts, visitor headers, consent/rate-limit/rejected requests,
  anonymous empty Home and radio outage fallback. Live endpoint evidence and
  remaining service/account/device boundaries are recorded in docs/VALIDATION.md.

### Rust-only application and general settings (M1/M2/M3/M5/M7/M8)

- Committed docs/REMAINING_WORK.md before removing legacy Python source, tests,
  themes, packaging and development tools. Existing installed files/data remain
  intact; migration and Git history preserve the compatibility path.
- Default installer builds Rust and publishes both `ytui` and `ytui-rs` after
  build/version verification. A failed build preserves existing launchers; a
  launcher symlink is replaced without overwriting its target. Compatibility
  install-rust.sh calls the same installer. Alpha version is 0.2.0-alpha.4.
- Added eight settings categories and 50 editable fields with choices, switches,
  keyboard/mouse Save/Cancel, draft validation and persistent configuration.
  Settings tests cover 1,040 additional theme/size/first-last-field combinations.
  Frame pacing coalesces burst input; reduced motion limits redraw frequency.
- Mapped legacy audio quality, playlist and motion preferences, and list retained
  unsupported fields. Added Music playlists search and CLI category/details/
  related routes, plus clear player-unavailable reasons.
- Replaced Python capture/check scripts with a native Rust PTY/font renderer and
  PNG/manifest integration check. Generated 43 WQHD captures, including every
  theme, settings category and additional popups. Real terminal checks cover
  popup navigation, Unicode input, resize and terminal restoration.
- 85 default tests pass; strict Clippy/formatting and a separate real-FFmpeg
  fixture pass. Four opt-in public YouTube tests were run and failed on HTTP/
  proxy access; they do not satisfy live account/native playback acceptance.

### Agent handoff and completion evidence

- Read this plan and `AGENTS.md` before changes. Use milestone IDs in PRs and
  report claimed scope, changed contracts, checks, and remaining limitations.
- Reserve crate/API contracts before independent work. Suggested work areas:
  shell/input, download/process/persistence, browse/auth/images, music/playback,
  radio, themes, and integration/review. This describes ownership boundaries;
  it does not require spawning agents or changing files concurrently.
- Keep shared model changes in small prerequisite PRs. Feature workers own
  adapters and their fixtures; an integrator verifies cross-tab behavior and
  installer/migration. Do not hide errors behind empty lists or swallow task exits.
- Append a milestone status/evidence entry here when complete. Checkboxes or
  tables only change to done with named tests/artifacts or a documented manual
  acceptance result. Reviewers should distinguish fixtures from live-service tests.

## Validation and release criteria

Default automated checks: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, and `cargo build --release --locked`. Feature-specific
checks must accompany optional dependencies. CI must not need a browser account,
audio device, desktop bus, internet radio stream, or live YouTube response.

Meaningful tests:

- State-machine sequences and fake child tools: remove during metadata, pause
  during conversion, retry a cancelled job, out-of-order progress, process crash,
  pipe backpressure, partial JSON, final path changes, and bounded shutdown.
- Restart/migration fixtures: interrupted jobs, repeated migration, malformed or
  older configs, Unicode filenames, playlist order, duplicate detection, and a
  second process attempting to own the same queue.
- Sanitized recorded provider fixtures: continuation pages, missing fields,
  deleted videos, expired sessions, multiple accounts, and account cache isolation.
  Use synthetic secrets for auth tests; never commit real browser fixtures.
- TUI reducer/input tests plus a small set of render snapshots for focus, narrow
  layouts, modals, loading/errors and light/dark themes. PTY smoke tests restore
  raw mode/alternate screen on normal exit and errors.
- Theme fixtures for current/legacy Omarchy, Caelestia, portal changes, atomic
  replacements, missing desktop services, invalid colors and manual overrides.
- Playback/radio mocks for short previews, queue transitions, live vs finite media,
  M3U/PLS failures, IPC disconnect and reconnect. Check that secrets never reach
  captured subprocess arguments or logs.

Manual acceptance on CachyOS + Caelestia must include:

1. Fresh start; anonymous search; thumbnail display/fallback; keyboard-only
   browsing; resizing during loading; tab switches during active downloads.
2. Brave Origin browser/profile import, correct account/channel,
   Home/subscriptions/Watch Later, search → details → preview → play → download.
3. Video/audio/playlist download, merge/conversion, pause/resume, remove while
   resolving, cancel while converting, failed network, disk/path errors and restart.
4. Music library → artist/album → playback queue; next/previous/seek/shuffle/repeat.
5. Cliamp catalog plus a Radio Browser station; favorites/import; broken stream;
   return from optional cliamp launch with terminal and playback state intact.
6. Live Caelestia palette changes, explicit theme override, small terminal and
   unsupported-image fallback; separately test Omarchy and GNOME environments.
7. Upgrade a copy of Python state, verify files/history/queue, then roll back.

Initial responsiveness budgets: input feedback within 100 ms in a loaded UI,
no network wait on the render thread, a visible cancel response within 100 ms,
and owned child-process termination/reaping within a bounded 5-second deadline
before any forced cleanup. Measure on documented hardware; slow network media
startup is shown as loading and is not disguised by these UI targets.

## Risks and decisions to keep visible

- YouTube's internal browsing APIs and stream challenges change. Isolate adapters,
  retain sanitized fixtures, pin a known-working dependency range, provide clear
  diagnostic/update guidance, and never promise permanent API compatibility.
- Personal Home/library access must pass an authenticated spike; do not postpone
  it until after the UI is built. Live account/media validation cannot be completed
  in this cloud environment without the user's local browser and desktop.
- Terminal graphics vary across terminal/tmux/SSH combinations; maintain a useful
  no-image path. Full moving video in a terminal is not a baseline promise.
- The user confirmed native radio plus optional cliamp launch and short muted
  previews on `v`. Verify Ghostty graphics and Brave Origin cookie extraction in
  the primary acceptance environment before calling either supported.
- Feature ideas after the core is reliable: MPRIS/playerctl/media keys, SponsorBlock
  playback controls, local offline library, resume positions, lyrics, and exportable
  playlists. Preserve existing SponsorBlock download settings in the parity review;
  new optional features must not delay basic reliability.

## Research and attribution

Primary sources inspected on 2026-10-21; verify upstream revisions before reuse.
Inspiration is not an instruction to copy a whole application. Retain notices and
check each dependency or copied component's actual license before incorporation.

- [Current ytui source](https://github.com/Vanir20342222/ytui/tree/4bdee78d463740ed959a3e69da7b8c314be992e6): baseline features and lifecycle findings above.
- [yt-dlp](https://github.com/yt-dlp/yt-dlp#readme): extraction, browser-cookie options,
  progress templates, postprocessing and final-output reporting.
- [youtui / ytmapi-rs](https://github.com/nick42d/youtui) and
  [ytmapi-rs manifest](https://github.com/nick42d/youtui/blob/main/ytmapi-rs/Cargo.toml):
  Rust Music API candidate and artist/album navigation inspiration.
- [ytmusic-tui](https://github.com/WakaTaira/ytmusic-tui): browser-auth setup,
  shared playback/navigation ideas; its documented flow validates before saving.
- [rookie](https://github.com/thewh1teagle/rookie): Rust browser-cookie extraction
  candidate; platform/profile support still needs testing for this app.
- [youtube-tui](https://github.com/Siriusmart/youtube-tui): terminal browsing and
  thumbnail UX inspiration, not a personalized-feed implementation guarantee.
- [cliamp radio documentation](https://github.com/bjarneo/cliamp/blob/main/docs/radio.md)
  and [Go module](https://github.com/bjarneo/cliamp/blob/main/go.mod): radio sources,
  live vs track-library channels, and process-integration boundary.
- [Caelestia Colours.qml](https://github.com/caelestia-dots/shell/blob/main/services/Colours.qml)
  and [Paths.qml](https://github.com/caelestia-dots/shell/blob/main/utils/Paths.qml):
  actual scheme file structure, watched file, and XDG state path.
- [Omarchy theme manual](https://omarchy.org/manual/making-your-own-theme/) and
  [upstream theming flow](https://github.com/omacom/omarchy/blob/quattro/docs/theming.md):
  palette schema, mode, staged replacements, current state location and older compatibility.
- [XDG Settings portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Settings.html):
  desktop appearance hints and change notifications.

## Alpha.5 follow-up (M3–M8)

See [the coordinated playback/navigation plan](docs/PLAYBACK_NAVIGATION_PLAN.md)
for the user report, confirmed auto fallback change, implementation and evidence.
The bottom command entry is replaced by Actions/forms. Native upstream restrictions
and failed live checks remain visible instead of being described as passed features.

### M4/M5 follow-up: automatic feed recovery (alpha.6)

The user reports having to refresh YouTube feeds manually. Transient load and
continuation failures now retry automatically with bounded request duration and
backoff, preserving visible content. Fresh HTML Home data avoids an API request;
client contexts renew, and overlapping pages can advance.
See [FEED_RECOVERY_PLAN.md](docs/FEED_RECOVERY_PLAN.md) for behavior and checks.
Successful live account/desktop acceptance remains required.

### M5/M8 follow-up: playback fallback repair (alpha.7)

User reports every video failing after the fallback indicator with mpv 0.41.0 /
yt-dlp 2026.08.19. External playback now resolves streams directly with yt-dlp,
forwards the extractor HTTP identity and separate audio, and launches mpv without
its extraction hook. Capture stdout/stderr and preserve specific extraction/open
errors. Real mpv loopback split-video/audio and audio-only playback is covered,
alongside default regressions. See [PLAYBACK_REPAIR.md](docs/PLAYBACK_REPAIR.md)
and [VALIDATION.md](docs/VALIDATION.md) for actual evidence and live boundaries.

The same follow-up covers M3: dedicated playlist intent, original collection source
preservation, subset/import/dedupe handling and retained completed files on partial
failure. Native YouTube playlists request the supported auto fallback before any
transfer. A real FFmpeg MP3 download/conversion and loopback HTTP-rejection fallback
complement synthetic three-entry playlist checks. Old jobs whose collection URL was
already replaced need the original playlist re-added; downloaded files are preserved.

The requested test expansion adds eleven default regressions for failure/retry/
cancellation/tool metadata and player recovery, plus real M4A/Opus/FLAC conversion
and mpv resume/preview checks. Tests reproduced and fixed per-file hook metadata,
duplicate invocation and missing output records on hook failure (M3/M8). Final
counts and reproducible commands are in docs/VALIDATION.md.
