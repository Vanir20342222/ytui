# Rewrite parity and release gate

The Rust app is an alpha implementation, not a declaration that all bugs or all
milestones have passed. The Python application/installer remain intact. Native
YouTube extraction has the explicit upstream limitations documented in RUST.md.

| Area | Rust implementation | Remaining release work |
| --- | --- | --- |
| Four-tab shell, Unicode input, focus, help, back navigation | Implemented; size/input tests | Ghostty/tmux graphical inspection |
| Download queue, pause/cancel/remove/retry/reorder | Implemented; generations reject stale results | Long real-media and disk-full tests on CachyOS |
| Native download/playback extraction | Pinned yt-dlp-rs, isolated worker; default | Many YouTube videos/playlists/live/resume unsupported upstream; explicit fallback retained |
| yt-dlp fallback | Explicit per-job/action/CLI/global selection | Real YouTube challenge/version checks |
| Format/container, conversion, metadata, cover art | Implemented | Broad codec/container fixture coverage |
| Subtitles | Native literal-language selection, manual/automatic sidecars and embedding; explicit fallback also supported | Live captions and broader codec/container acceptance (real FFmpeg fixture passes) |
| SponsorBlock, chapter split, bandwidth | Implemented in explicit fallback; native rejects unsupported options | Native adapters remain incomplete upstream/in ytui |
| Local conversion | Separate FFmpeg output; input preserved | Broad real-codec checks |
| Search and personalized YouTube feeds | Rust internal-API adapter; current and legacy renderer parsing | Live signed-in home/subscriptions/Watch Later validation |
| Music home/search/library/album/artist/playlist | Dedicated WEB_REMIX requests; category commands | Live signed-in response/continuation validation |
| Playback queue, shuffle/repeat/seek/volume | Implemented with mpv IPC | Real audio/video device acceptance |
| Muted preview and restoration | Implemented; fake mpv IPC/argument test | Window behavior and rapid switching in Ghostty/Hyprland |
| Cookie/profile import | Native rookie, Origin directory discovery, scoped session validation | Actual Brave Origin keyring/decryption on CachyOS |
| Keyring/file storage, disconnect, account index | Implemented; synthetic auth tests | Live account/channel switching and portal/keyring errors |
| Cliamp streams and track libraries | Native API adapter, favorites/custom import, optional external cliamp | Live audio acceptance |
| Radio Browser | Search/country/tag, country pins, failover, cached catalogs, playlist resolution, reconnect | Real audio acceptance; location suggestions are optional and not implemented |
| Curated and system themes | Caelestia/Omarchy/portal/GNOME, live updates, manual override, ANSI | GNOME/Omarchy desktop sessions and Ghostty graphics |
| Persistence/migration/history/export/profiles | Separate DB, group expansion, bulk loaded-page ranges, JSON/text import, complete native profile/keymap roundtrip; raw legacy settings/keys/profiles preserved | Unsupported legacy fields are preserved, not applied; real-data upgrade acceptance |
| Old slash-command aliases/panels | Queue/history/stats/mode/profiles/settings/pause/stop/import/export aliases, key editor and scrollable help | `/update` and `/log`; full legacy panel presentation |
| VPN/Proton/WireGuard | Explicit NetworkManager VPN/WireGuard list/connect/disconnect via supervised nmcli; no startup probing | Standalone Proton CLI/login/server selection, raw wg-quick and non-Linux providers remain open; real NetworkManager acceptance |
| Scheduling and off-peak bandwidth | Persistent delayed jobs, daily local-time windows, rate windows and safe stop/requeue on boundaries; CLI explicit override | Real long-transfer/timezone/DST acceptance; recurring watch subscriptions absent |
| Clipboard polling | Opt-in bounded Wayland/X11 polling with baseline/change detection and a review action | Actual Ghostty/Caelestia clipboard acceptance |
| Desktop notifications and sounds | Native D-Bus completion/failure notifications; optional sound hints | Real notification server/sound acceptance (daemon may ignore hints) |
| Statistics/history filters | Dedicated searchable/filterable panel with recorded transfer/media totals | Broader statistics and physical disk inventory are optional |
| MPRIS, lyrics, offline library | Optional feature ideas | Not required for this alpha; separate work |
| Packaging/installer | Rust source build + separate ytui-rs installer | Default replacement/release packages only after target acceptance |

`clipboard_watcher`, `notifications` and `notification_sound` now have Rust effects
and working Settings controls. Clipboard watching defaults off and never starts a
download without review. Legacy values without native equivalents remain in both
the original backups and the profile's verbatim legacy JSON. No real clipboard,
notification server, VPN or desktop theme is changed/read by automated tests;
process and auth tests use synthetic tools/data.

Replacing the default installer still requires the plan's parity/scope decision
and target-machine acceptance. This branch does not remove the Python fallback
application or claim those gates have passed.
