# Rust-only transition: gaps and acceptance

This inventory was written **before removing the Python application**, following
the user's explicit Rust-only transition request on 2026-10-21. Git retains the
old implementation and its tests; installed user files, settings, queues and
history must stay intact. Rust migration remains available. Removal of the old
app does not mean the live-service or desktop release gates have passed.

The detailed feature/verification inventory is [FEATURE_AUDIT.md](FEATURE_AUDIT.md).
[PARITY.md](PARITY.md) maps the implemented Rust equivalents. This document
lists remaining gaps, including options preserved verbatim by migration.

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

## Unsupported or partial functionality

| Area | Current limit / planned acceptance |
| --- | --- |
| Native YouTube extractor | Pinned upstream has incomplete current-video/challenge/live/playlist support. Native transfer pause starts again; persistent range resume is partial upstream functionality. User-confirmed auto fallback is implemented for supported extractor limitations; strict native mode remains available. Successful live playback is still unverified here. |
| Native subtitles | Literal languages, automatic/manual captions, sidecars and embedding exist. Regex selectors, configurable subtitle conversion format, and automatic translation to a target language are not implemented. Cuts with subtitles require explicit fallback to prevent desynchronization. |
| Video selection | Resolution/container are supported. A separate prefer-high-frame-rate control and Python's automatic video-conversion policy are not implemented. Local conversion remains explicit and preserves the source. |
| Destination convenience | Video/audio directories exist. Recent-directory history and automatic opening of completed files/folders are not implemented; explicit open/play remains available. |
| Appearance | Adaptive layout, themes, images, reduced-motion config exist. Separate icon-style and density preferences, startup ASCII/boot effects and arbitrary legacy Textual styles are not ported. |
| VPN | Existing NetworkManager VPN/WireGuard profiles can be listed/connected/disconnected explicitly. Standalone Proton login/server selection, automatic reconnect monitoring, raw wg-quick and non-Linux adapters are not implemented. No startup VPN mutations. |
| YouTube account features | Home/subscriptions/library/liked/later/history are implemented through a read-only internal API, but live signed-in correctness, brand channels, expiry and Brave Origin keyring decryption need target testing. Account mutations and automatic watch-history upload are intentionally absent. |
| Music | Dedicated WEB_REMIX browsing/search/library exists; live continuations/account journeys need verification. Lyrics, MPRIS/media keys and track-based radio are not implemented. |
| Cliamp | Native catalog, track libraries and optional external launch exist. The live catalog and Lofi track library were reachable during alpha.5 work; real stream/audio acceptance remains open. A labelled Radio Browser fallback is available. This is not a full port of cliamp's unrelated providers. |
| Playback tracks | Audio/subtitle cycling operates on tracks loaded by mpv. Native URL-only resolution does not yet enumerate every alternative YouTube language/caption track; full website-style track discovery remains open. Real mpv headless controls are checked; displayed window/device acceptance remains open. |
| Radio | Directory search/country/tag, favorites, custom stations, playlist resolution and reconnect exist. Automatic location suggestions are absent. Real stream metadata/reconnect/audio-device behavior needs testing. |
| Hooks/notifications/clipboard | Native implementations exist and fixture tests pass. Real Wayland/X11 clipboard, D-Bus sound hints and arbitrary user hooks need local acceptance. Hooks stay opt-in. |
| Scheduling/watches | Download/bandwidth windows and collection watches exist. Large collection bounds are explicit; long-transfer DST/timezone behavior and live account polling need acceptance. |
| History/statistics | Search/filter/group and recorded totals exist. Full disk inventory and charts are optional, not implemented. |
| Updates/packaging | Source build is supported. Automatic in-place binary updates, signed release archives, distribution packages and non-Linux packaging are not implemented. Rust replaces the repository app at the user's request, without a stable-release claim. |
| Language/localization | English UI only; the original app also shipped only English translations. |
| First run / legacy connection fields | Setup is through F2 → Account, `init` and `doctor`, without an automatic welcome wizard. Legacy `first_run_done` is retained but unused. Legacy browser/cookie path fields do not trigger automatic credential reads; import explicitly. Proxy type comes from the URL scheme rather than a separate legacy type field. |
| Bulk selection | Loaded-page ranges and explicit playlist download options exist. Selecting across an entire remote collection depends on fetching its pages; native upstream playlist extraction remains limited. |

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
