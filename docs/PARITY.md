# Rewrite parity and release gate

The Rust app is an alpha implementation, not a declaration that all bugs or all
milestones have passed. The user authorized the Rust-only repository transition
after [the gap inventory](REMAINING_WORK.md) was committed. Native
YouTube extraction has the explicit upstream limitations documented in RUST.md.

| Area | Rust implementation | Remaining release work |
| --- | --- | --- |
| Four-tab shell, Unicode input, focus, help, back navigation | Adaptive catalog/details, tab-specific feed picker, remapped hints, compact player/dialogs and size/input tests | Ghostty/tmux graphical inspection |
| Download queue, pause/cancel/remove/retry/reorder | Implemented; generations reject stale results | Long real-media and disk-full tests on CachyOS |
| Native download/playback extraction | Pinned yt-dlp-rs, isolated worker; auto tries native first | Many YouTube videos/playlists/live/resume unsupported upstream; user-confirmed automatic fallback plus strict native retained |
| yt-dlp fallback | Auto for supported limitations plus explicit per-job/action/CLI/global selection | Real YouTube challenge/version checks |
| Format/container, conversion, metadata, cover art | Implemented | Broad codec/container fixture coverage |
| Subtitles | Native literal-language selection, manual/automatic sidecars and embedding; explicit fallback also supported | Live captions and broader codec/container acceptance (real FFmpeg fixture passes) |
| SponsorBlock, chapter split, bandwidth | Native pacing, chapter parsing/embedding/splitting and SponsorBlock mark/cut adapters; explicit fallback also supported | Live YouTube/API and broader codec coverage; cuts with subtitles require fallback; chapter copies can round to keyframes |
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
| Old slash-command aliases/panels | Queue/history/stats/mode/profiles/settings/pause/stop/import/export aliases, key editor, diagnostics and update instructions | Full legacy panel presentation; native updates use reviewed builds |
| VPN/Proton/WireGuard | Explicit NetworkManager VPN/WireGuard list/connect/disconnect via supervised nmcli; no startup probing | Standalone Proton CLI/login/server selection, raw wg-quick and non-Linux providers remain open; real NetworkManager acceptance |
| Scheduling and off-peak bandwidth | Persistent delayed jobs/windows, rate caps, recurring collection watches with baseline/backoff/account invalidation | Real long-transfer/timezone/DST and live collection acceptance; large watch catalogs have explicit bounds |
| Clipboard polling | Opt-in bounded Wayland/X11 polling with baseline/change detection and a review action | Actual Ghostty/Caelestia clipboard acceptance |
| Desktop notifications and sounds | Native D-Bus completion/failure notifications; optional sound hints | Real notification server/sound acceptance (daemon may ignore hints) |
| Statistics/history filters | Dedicated searchable/filterable panel with recorded transfer/media totals | Broader statistics and physical disk inventory are optional |
| Offline library and local resume | Existing completed-media catalog, explicit local resume, private bounded positions and `--offline` mode | Actual mpv/device acceptance and additional codec/filesystem coverage |
| MPRIS and lyrics | Optional feature ideas | Not required for this alpha; separate work |
| General settings | Nine categories, 53 fields, choices, switches, mouse Save/Cancel and validated persistence; every category/size/theme tested | Real desktop preferences and account/keyring acceptance |
| Packaging/installer | Default Rust installer publishes both ytui and ytui-rs; old Python sources/package/tests removed; Rust screenshot tooling | Signed release packages and stable desktop acceptance |

`clipboard_watcher`, `notifications` and `notification_sound` now have Rust effects
and working Settings controls. Clipboard watching defaults off and never starts a
download without review. Legacy values without native equivalents remain in both
the original backups and the profile's verbatim legacy JSON. No real clipboard,
notification server, VPN or desktop theme is changed/read by automated tests;
process and auth tests use synthetic tools/data.

Native postprocessing is fixture-tested, including a real FFmpeg mark/cut/split
journey; it is not evidence of unrestricted native YouTube extraction. Rust state
version 2 upgrades retain a version-1 backup. Forced exits can leave recoverable
staging directories; only stale owned credential exports/manifests are cleaned
automatically after the instance lock is acquired.

The default installer and repository application are Rust at the user's explicit
request. Live desktop/account release gates remain open. The optional external
yt-dlp executable is retained as the previously requested explicit fallback.

## Alpha.5 navigation/playback update

- F2 Actions and parameter forms replace the bottom command entry; nine settings
  sections / 53 fields and three download sections expose the controls.
- Size-aware memory/disk thumbnails, nearby prefetch, section/category parsing and
  background continuation keep browsing selection stable.
- Persistent volume/mute/speed, batched IPC, direct live station selection, song
  continuation, player/timeline controls and a bundled mpv window menu are implemented.
- Read-only playback SponsorBlock provides marks, configurable skip and undo without
  cutting source media. Direct extraction does not run the website ad player.
- Current live and desktop acceptance boundaries are in VALIDATION.md; no claim of
  full YouTube website/player parity or all bugs fixed is made.
