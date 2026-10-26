# Rust-only transition: gaps and acceptance

This inventory was written **before removing the Python application**, following
the user's explicit Rust-only transition request on 2026-10-21. Git retains the
old implementation and its tests; installed user files, settings, queues and
history must stay intact. Rust migration remains available. Removal of the old
app does not mean the live-service or desktop release gates have passed.

The detailed feature/verification inventory is [FEATURE_AUDIT.md](FEATURE_AUDIT.md).
[PARITY.md](PARITY.md) maps the implemented Rust equivalents. This document
lists remaining gaps, including options preserved verbatim by migration.

## Current candidate: alpha.12

[Recovery fixes after local acceptance](RECOVERY_FIXES_2026-10-26.md) repair
rate-limit retries, live EOF/backend/control recovery and archived-live metadata.
The new loopback real-player fixture is separate from active VPN-loss/recovery,
human audibility and the remaining anonymous-service acceptance on the actual PC.

[The October 26 target-PC report](LOCAL_ACCEPTANCE_2026-10-26.md) adds actual
Brave Origin import/feeds/expiry, authenticated two-file playlist, Ghostty
thumbnails/Omarchy palette and native desktop completion evidence. The current
host runs restored Omarchy. Anonymous Music/playlist failures, a 2/3 native
corpus with HTTP 429, human audio acceptance, active-stream VPN recovery and
representative reliability remain open; older results below retain their dates.

[The audit implementation tracker](AUDIT_IMPLEMENTATION.md) supersedes the older
implementation-gap rows below for formats, account setup, MPRIS, collection groups,
recent folders, file opening, diagnostics and release tooling. Their live/desktop/
packaging acceptance remains explicitly separate. The follow-up adds reviewed
LRCLIB/local LRC lyrics and initial German navigation/settings/actions/help;
remaining status/guidance prose is not fully localized. The alpha candidate is
integrated on main; a tagged stable release still requires the acceptance gates.

The [researched feature follow-up](FEATURE_RESEARCH_2026-10-24.md) adds a local
saved-video/song shelf and session sleep timer. Saved links can be reviewed
offline and survive account disconnect; they require a connection to play.
This does not implement named local playlists or remote account mutations.

## Previous local evidence (alpha.11)

The transition and alpha sections below retain their original results. Later
target-PC runs verified public native playback/downloads, provider caching, radio
catalogs/streams and playback fault recovery. One complete muted gpu-next/PipeWire
desktop session also passed. See [the playback sweep](SINGLE_VIDEO_SWEEP.md) and
[network report](NETWORK_PERFORMANCE.md) for exact scope. Human visual/audible
acceptance, signed-in journeys, broader media coverage and the requested 99%
reliability remain open. Hosted-runner extraction failures are recorded separately
in [the CI diagnosis](CI_PLAYBACK_DIAGNOSIS.md).

## Work in this transition

| Gap recorded before removal | Result |
| --- | --- |
| General settings is a long flat form with hidden configuration options | Implemented eight sections and 50 fields, choice controls, keyboard/mouse Save/Cancel, validation and persistence. All sections/ends/themes/sizes have coverage. |
| Python source, packaging and installer are still the default | Removed the old app/tests/package metadata. Default installer builds Rust and publishes both launchers; migration remains available. |
| Screenshot generation and PNG verification use Python tooling | Replaced by a Rust PTY/font renderer and Rust PNG/manifest test; 43 WQHD captures generated. |
| Migration does not apply legacy audio quality, playlist mode or reduced-motion preference | Mapped these values. Unsupported settings are retained and identified in the migration report. |
| Music category search lacks a playlists category | Implemented `:playlists QUERY`, the saved-playlist route, and headless Music category commands. Live API validation remains blocked. |
| Tests cover rendering but don't exercise every popup's interactions | Added real-executable PTY popup/resize/terminal-restoration coverage and settings edit/error/save/cancel/mouse/persistence tests. See the popup inventory in VALIDATION.md. |
| Live YouTube checks are blocked here | Ran four opt-in tests: public search, Music search, details and native test-video download. All four failed during HTTP/proxy access. Signed-in/browser/device acceptance stays pending. |

## Alpha.5 follow-up

Playback/navigation changes are described in [PLAYBACK_NAVIGATION_PLAN.md](PLAYBACK_NAVIGATION_PLAN.md)
and [RUST.md](RUST.md): auto extraction fallback, cache/prefetch, server categories,
continuous feed loading, Actions/forms, persistent player preferences and reversible
playback SponsorBlock. General settings now has nine sections / 53 fields.
No mandatory live-service/desktop gate is removed by these implementations.

## Alpha.6 feed recovery (M4/M5)

Temporary YouTube/Music failures recover automatically with backoff/Retry-After,
active-tab scheduling and stale-result protection. Fresh embedded Home data avoids
an API round trip; stale contexts and bare Home shells renew once. Loaded content
and same-feed selection survive refresh/recovery. Overlapping continuation pages
advance with token-cycle/empty-run bounds. See [FEED_RECOVERY_PLAN.md](FEED_RECOVERY_PLAN.md).
This does not remove consent, expired-session, service-block or live-account gates.

## Alpha.7 playback fallback repair (M5/M8)

The user reports all videos failing after fallback with mpv 0.41.0 and yt-dlp
2026.08.19. External playback now resolves through yt-dlp directly and hands streams,
headers and separate audio to mpv. Actual mpv errors are captured from stdout/stderr.
See [PLAYBACK_REPAIR.md](PLAYBACK_REPAIR.md). This fixes the hidden-error path and
removes the mpv extraction-hook dependency; successful live YouTube playback on the
reported desktop remains an acceptance gate. Dedicated playlist intent and saved
source/ranges now survive entry metadata/import/retry; native YouTube playlists use
auto fallback. Partial output records do not turn failed collections into success.
A real native audio transfer/MP3 conversion is verified with loopback/FFmpeg; actual
YouTube Music and playlists still require live acceptance. Re-add original playlist
links for older jobs already reduced to a single-song URL.

Live follow-up on 2026-10-22: GitHub runners reached public YouTube/Music and both
searches passed through ytui. In both auto and yt-dlp modes, the other seven live
checks failed with YouTube's bot-confirmation request. See [LIVE_YOUTUBE_TESTS.md](LIVE_YOUTUBE_TESTS.md)
for the executable local runner and actual CI evidence. Playback, downloads,
two-entry playlists and authenticated/device acceptance remain open.

## Unsupported or partial functionality

Alpha.9 updates native extraction and makes playback native by default, separately
from download fallback. HLS manifests, native HTTP identities and installed Node
challenge solving have local actual-tool coverage; live watching and the requested
99% reliability remain open. See [NATIVE_PLAYBACK.md](NATIVE_PLAYBACK.md).

Alpha.8 closes additional local output/preview/diagnostic defects and adds an
actual yt-dlp two-file playlist regression over loopback. It does not remove the
live acceptance gates above. See [STABILITY_PASS.md](STABILITY_PASS.md).

| Area | Current limit / planned acceptance |
| --- | --- |
| Interrupted playlists | Completed files stay on disk; per-entry durable checkpoints during cancellation are partial. Strict generations still reject late updates from stopped workers. Retry/library discovery can recover existing files. |
| Native YouTube extractor | Updated pinned upstream supports Node/Deno challenges and more clients; ytui preserves native HLS and stream headers. Native-default playback never automatically invokes Python. Login/live/DRM and representative live watching remain unverified or limited; 99% success is not established. Download auto fallback remains separate. Native transfer pause starts again; persistent range resume is partial upstream functionality. |
| Native subtitles | Literal languages, automatic/manual captions, sidecars and embedding exist. Regex selectors, configurable subtitle conversion format, and automatic translation to a target language are not implemented. Cuts with subtitles require explicit fallback to prevent desynchronization. |
| Video selection | Resolution/container are supported. A separate prefer-high-frame-rate control and Python's automatic video-conversion policy are not implemented. Local conversion remains explicit and preserves the source. |
| Destination convenience | Video/audio directories and bounded recent destinations exist. Recorded files can be explicitly opened through the desktop portal; automatic opening remains absent. Physical portal acceptance remains open. |
| Appearance | Adaptive layout, themes, images, reduced-motion config exist. Separate icon-style and density preferences, startup ASCII/boot effects and arbitrary legacy Textual styles are not ported. |
| VPN | Existing NetworkManager VPN/WireGuard profiles can be listed/connected/disconnected explicitly. Standalone Proton login/server selection, automatic reconnect monitoring, raw wg-quick and non-Linux adapters are not implemented. No startup VPN mutations. |
| YouTube account features | Home/subscriptions/library/liked/later/history are implemented through a read-only internal API, but live signed-in correctness, brand channels, expiry and Brave Origin keyring decryption need target testing. Account mutations and automatic watch-history upload are intentionally absent. |
| Music | Dedicated WEB_REMIX browsing/search/library, reviewed LRCLIB/local lyrics and MPRIS exist. Live continuations/account journeys, desktop media keys, real lyric matching and track-based radio need qualification or further work. |
| Cliamp | Native catalog, track libraries and optional external launch exist. Public catalog/library and real headless station playback passed locally; physical audio acceptance remains open. A labelled Radio Browser fallback is available. This is not a full port of cliamp's unrelated providers. |
| Playback tracks | Actual extractor formats/languages/captions and loaded mpv tracks now have a picker. Every YouTube alternative is not guaranteed; source/backend caption and track availability still needs live qualification. Real headless controls and one muted desktop completion pass; human visual and audible-device acceptance remains open. |
| Radio | Directory search/country/tag, favorites, custom stations, playlist resolution and reconnect exist. Public catalog/filter and headless stream checks passed locally. Automatic location suggestions are absent; broader live metadata/reconnect and physical audio acceptance remain open. |
| Hooks/notifications/clipboard | Native implementations exist and fixture tests pass. Real Wayland/X11 clipboard, D-Bus sound hints and arbitrary user hooks need local acceptance. Hooks stay opt-in. |
| Scheduling/watches | Download/bandwidth windows and collection watches exist. Large collection bounds are explicit; long-transfer DST/timezone behavior and live account polling need acceptance. |
| History/statistics | Search/filter/group and recorded totals exist. Full disk inventory and charts are optional, not implemented. |
| Updates/packaging | Source build is supported. Automatic in-place binary updates, signed release archives, distribution packages and non-Linux packaging are not implemented. Rust replaces the repository app at the user's request, without a stable-release claim. |
| Language/localization | Initial German UI and en/de/system setting now exist; remaining guidance/status prose and tool/provider output retain their original language. See AUDIT_FEATURES.md. |
| First run / legacy connection fields | Setup is through F2 → Account, `init` and `doctor`, without an automatic welcome wizard. Legacy `first_run_done` is retained but unused. Legacy browser/cookie path fields do not trigger automatic credential reads; import explicitly. Proxy type comes from the URL scheme rather than a separate legacy type field. |
| Bulk selection | Loaded-page ranges and explicit playlist download options exist. Explicit Rust-browser full-collection expansion fetches every page within documented limits and queues per-entry jobs. Incomplete expansion queues nothing; native upstream whole-playlist extraction remains guarded. |

## Data compatibility

Legacy settings, keybindings, profiles and SQLite queues/history are read only
when the user requests migration. Originals and unmapped JSON values are retained.
Unsupported settings above are preserved, not silently treated as implemented.
Deleting tracked Python source never deletes the user's installed virtualenv,
downloaded files, browser profiles or data directories. The old source remains
recoverable from Git history.

## Test boundaries

Default tests use synthetic sessions/tools/providers and isolated paths. Live
public probes are opt-in. Tests never import real browser cookies, accept consent,
change a VPN or change desktop themes. Actual CachyOS + Caelestia + Ghostty +
Brave Origin + Proton VPN acceptance remains a separate gate when this workspace
cannot reach YouTube or access the user's desktop/audio devices.
