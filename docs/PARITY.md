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
| Subtitles, SponsorBlock, chapter split, bandwidth | Implemented in fallback; native rejects unsupported options | Native adapters for these upstream APIs/features |
| Local conversion | Separate FFmpeg output; input preserved | Broad real-codec checks |
| Search and personalized YouTube feeds | Rust internal-API adapter; current and legacy renderer parsing | Live signed-in home/subscriptions/Watch Later validation |
| Music home/search/library/album/artist/playlist | Dedicated WEB_REMIX requests; category commands | Live signed-in response/continuation validation |
| Playback queue, shuffle/repeat/seek/volume | Implemented with mpv IPC | Real audio/video device acceptance |
| Muted preview and restoration | Implemented; fake mpv IPC/argument test | Window behavior and rapid switching in Ghostty/Hyprland |
| Cookie/profile import | Native rookie, Origin directory discovery, scoped session validation | Actual Brave Origin keyring/decryption on CachyOS |
| Keyring/file storage, disconnect, account index | Implemented; synthetic auth tests | Live account/channel switching and portal/keyring errors |
| Cliamp streams and track libraries | Native API adapter, favorites/custom import, optional external cliamp | Live audio acceptance |
| Radio Browser | Search/country/tag, failover, cached catalogs, playlist resolution, reconnect | Region pins and location opt-in |
| Curated and system themes | Caelestia/Omarchy/portal/GNOME, live updates, manual override, ANSI | GNOME/Omarchy desktop sessions and Ghostty graphics |
| Persistence/migration/history/export/profiles | Separate DB, backup/import and repeat safety | Playlist group expansion UI, complete legacy profile/keymap roundtrip |
| Old slash-command aliases/panels | Core flows replaced by tabs/forms/commands | Full legacy alias compatibility |
| VPN/Proton/WireGuard | Python feature remains available | Native explicit-action provider/CLI integration and user scope decision |
| Scheduled/off-peak bandwidth and clipboard polling | Python feature remains available | Rust scheduler and opt-in polling; not yet implemented |
| Desktop notifications and notification sounds | Not implemented in Rust | Explicit local notification service adapter |
| Advanced statistics/history filters | Persistent completed rows and JSON export | Dedicated filtered history/statistics view |
| MPRIS, lyrics, offline library | Optional feature ideas | Not required for this alpha; separate work |
| Packaging/installer | Rust source build + separate ytui-rs installer | Default replacement/release packages only after target acceptance |

`clipboard_watcher` and `notifications` are retained configuration fields for the
parity inventory and currently have no Rust effect. They are not exposed as working
controls in the settings form. Deferred legacy settings remain in migration
backups. No VPN or system-theme command is executed during startup or tests.

Replacing the default installer still requires the plan's parity/scope decision
and target-machine acceptance. This branch does not remove the Python fallback
application or claim those gates have passed.
