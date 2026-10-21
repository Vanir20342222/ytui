# Feature audit — 2026-10-21

This inventory covers every implemented area in [PARITY.md](PARITY.md). A passing
fixture verifies the stated behavior; it does not certify a live YouTube account,
browser decryptor, desktop integration or audio device. This cloud workspace has
FFmpeg/ffprobe, but no mpv/cliamp installation, CachyOS desktop, Brave Origin
profile or signed-in Google account. The earlier public YouTube attempt failed at
the environment's network/proxy boundary. No credentials or private feeds were
used, and no networking restrictions were bypassed.

## Results

- **85 default tests pass**, with one real-media and four live-YouTube tests intentionally excluded.
- The separate real-FFmpeg/ffprobe mark/cut/split fixture passes.
- All **13 theme entries** render four workspaces and twelve dialogs at five
  sizes: 35×12, 80×24, 120×40, 180×50 and 212×58. All ten download phases are
  additionally checked in every theme. Text/selection contrast checks and ANSI
  color checks pass. See the [theme gallery](UI_THEMES.md).
- The actual binary passes demo/offline PTY checks, including theme switching,
  feed focus, Unicode input isolation, resize, saved-position/watch panels and
  clean terminal restoration. The capture tool verifies every PNG is 2560×1440.
- Formatting, strict Clippy and release builds pass. Build/installer evidence and
  exact environment boundaries remain in [VALIDATION.md](VALIDATION.md).

## Feature-by-feature coverage

| Feature | What was checked successfully | What remains unverified or unsupported |
| --- | --- | --- |
| Four workspaces and pane focus | Every theme/size; `b` focuses the wide feed pane, Enter chooses, Esc returns; compact picker; no playback on navigation | Actual Ghostty/tmux visual acceptance |
| General settings | Eight sections/50 fields, first/last-field rendering in 1,040 theme/size/selection states; real PTY navigation, keyboard/choice/mouse controls, invalid-save recovery, cancel and actual file persistence | Real account/keyring and desktop effects remain covered by their separate open gates |
| Keyboard, paste, mouse and resizing | Unicode editors; input isolation; remapping and occupied defaults; selection visibility; rendered mouse targets; clicks cannot pass through dialogs | Real terminal keyboard-layout variations |
| Theme selection and following | 13 render/capture variants; role contrast; ANSI chrome; synthetic Caelestia/Omarchy parsing and atomic replacement; last-valid recovery | Real GNOME/portal/Omarchy/Caelestia sessions; `system` gallery uses the built-in preview |
| Main/catalog/details/empty/error UI | Selected content, errors and controls remain visible; no preview action on stations/collections; all dialogs; ten queue phases | Subjective desktop review |
| Thumbnails | Real loopback PNG fetch/decode; disk-cache outage; corrupt data removal; byte/dimension limits; geometry, aspect and modal exclusion | Real YouTube CDN and Ghostty Kitty/Sixel behavior |
| Native downloads | Actual Rust CLI/helper transfer with Python executable absent; final output and byte checks; cancellable pacing | Upstream native YouTube coverage, long live transfers, disk-full/hardware errors |
| Explicit Python fallback | Fake executable contract; failure after 100% is not success; original outputs retained | Current real yt-dlp and YouTube challenges |
| Queue pause/retry/remove/reorder | Generation checks, deleted-row protection, restart recovery, cancellation/process ownership, UI/controller paths | Target-machine long-transfer stress |
| Output publication/recovery | No-clobber preflight/rollback; failed tagging retains a reported staged source | Filesystem-specific hard-link restrictions; abrupt power-loss acceptance |
| Audio/video options and forms | Audio/video/playlist mode selection reaches validated Add forms; destination toggle preserves custom edits; audio-quality validation | Broad real codec/quality combinations |
| Local conversion | Real FFmpeg WAV→MP3 smoke; input preserved; final extension/output; subprocess fixture tests | Broad codecs and unusual files |
| Subtitles | Native loopback sidecars, language errors and overwrite protection; real FFmpeg `mov_text` embedding with Python absent | Live YouTube caption formats; native SponsorBlock cuts with captions need explicit fallback |
| SponsorBlock | Synthetic API without Cookie header; untrusted/overlapping segments; real marking and one-second removal, verified duration/chapters | Live API reliability and broader codecs |
| Chapters | Description parsing/normalization, real embedding/splitting/retiming and unchanged original fixture | Live chapter metadata; stream-copy cuts can round to keyframes |
| Rate limits and daily windows | Actual rate timing/cancellation; global cap boundaries; midnight windows, state transitions and manual-pause preservation | Long transfers across DST/timezone changes |
| Collection watches | Complete first baseline queues nothing; deduplication, bounds, failure backoff, account invalidation and stale-result rejection | Actual signed-in collections, large catalog limits and long-running polling |
| YouTube home/search/subscriptions/later/liked/library/history | Current/legacy cards; complete loopback bootstrap redirects, separate contexts, visitor headers, search/continuation requests, 403/429 and consent instructions; anonymous Home guidance | **Live feed and account correctness remains open**; public probes fail at this environment's proxy tunnel |
| Channels/playlists/details/related | Provider parsing/contracts, collection navigation and selected details | Live internal-API responses and upstream playlist extraction limitations |
| YouTube Music home/search/library/albums/artists/playlists | WEB_REMIX adapter and synthetic album/song parsing without duplicates; scoped feed choices | **Live Music account/search/continuation acceptance remains open** |
| Browser import and credentials | Synthetic cookie scoping/redaction/file permissions, Netscape roundtrip and Origin directory discovery | **Actual Brave Origin decryption/keyring and Google validation remain open** |
| Account switching/disconnect | Controller invalidation and scoped state handling; no automatic history write-back | Real multi-account/brand-channel selection and desktop keyring failures |
| Video/audio playback and controls | Fake mpv IPC/argv for play, resume, pause, seek ±5, volume ±5 and mute; early close differs from EOF | **Actual mpv video/audio devices remain open** |
| Muted preview | Fake process/IPC verifies mute, ten seconds, cancel and prior playback restoration; preview-only UI | Actual windows/rapid switching on Hyprland/Ghostty |
| Playback queue/shuffle/repeat | Repeat Off/All/One, manual vs automatic advancement, previous, removals and nonrepeating shuffle; queue UI | Real device sequencing/reconnect stress |
| Local resume | Bounded/finite private positions, preview/live exclusions, explicit start argument and no startup playback; actual PTY saved-position panel | Real media/device acceptance |
| Offline mode/library | Catalog excludes missing/sidecar files; remote work/queue playback held; local search/resume/watch PTY; account loading skipped | Broader codecs/filesystems; user hooks retain their configured behavior |
| Cliamp channels/track libraries | Parser differentiates streams/libraries; current HTTP 503 triggers labelled directory fallback; loopback covers redirect/empty/outage/loop cases | The current catalog outage prevents live cliamp channel/library acceptance; actual playback and external cliamp remain open |
| Radio Browser | Actual alpha.3 CLI returns 80 public stations; concurrent server failover, query paths, pagination, no Cookie headers and disk-cache outage fixtures | Actual station playback and ongoing directory availability |
| Station playlists/live radio | Relative M3U and PLS, redirects, HLS pass-through, unsafe schemes and loop bounds | Actual stream metadata, reconnect timing and audio device |
| Favorites/custom stations/country pins | Controller state and parser validation; existing import/command paths | Real cliamp config variants and station persistence workflow acceptance |
| Persistence/state upgrade | SQLite locking, paused recovery, version-1 backup before version 2, permissions and transient cleanup | Upgrade a copy of the user's real state; power-loss acceptance |
| Migration/import/export/profiles/history/statistics | Atomic/idempotent import, group/filter indices, raw legacy preservation, config/keymap/profile roundtrips and actual panels | Real Python-state migration/rollback and deferred legacy fields |
| Clipboard/notifications | Synthetic clipboard exact-host/baseline/change checks, stale/disabled rejection and notification escaping | Actual Wayland/X11 clipboard, D-Bus delivery and sound |
| VPN controls | Supervised fake nmcli verifies escaped IDs and only explicit VPN operations | Real NetworkManager; standalone Proton/raw wg-quick are not implemented |
| Diagnostics/update instructions | Opt-in public `doctor --network` probes tested with actual release binary; host/cause retained with queries redacted; executable path/version and explicit branch/reinstall instructions | User's Proton exit and account probes remain open; distribution/package update workflows |
| Setup/installer/packaging | Default Rust installer and compatible alias; build failure preserves old launcher; symlinks replaced without touching legacy targets; both installed names report current version. Rust source/capture/PNG tests replace Python packaging/tools | Remote installer branch acceptance, signed binaries and stable release packaging |
| MPRIS, lyrics, location suggestions | Inventory reviewed | Optional ideas; not implemented or claimed working |

## Defects found and fixed during this audit

1. Mouse navigation used old, hard-coded tab coordinates. It now uses rectangles
   recorded by the actual renderer, including sidebar entries and scrolled lists.
2. Catppuccin Latte muted text and warning/success roles had weak contrast.
   Darker semantic variants now pass the documented contrast thresholds.
3. Image loading upscaled a 160×90 thumbnail to 1280×720. The decoder now retains
   native size unless a downscale is needed; the render protocol controls fitting.
4. Ordinary M3U wrappers rejected relative stream paths. They now resolve relative
   references against the wrapper URL while preserving HTTP(S)-only validation.
5. `init --config NEW_PATH` failed before creating its file, and saving a bare
   relative config path failed. Initialization now handles both and publishes
   without overwriting an existing or racing file.
6. Tiny layouts hid selected rows, long titles hid timing, and preview-only state
   looked idle. Responsive layout and explicit preview status cover these cases.

The remaining live checks above prevent an honest claim that every feature is
working on the target desktop. They are acceptance tasks, not passed tests.

Alpha.4 explicitly ran all four public YouTube smoke checks. Search, Music and
details failed at proxy tunnel setup; the native test-video download failed on its
HTTP request. These are recorded failures, not successful live-provider tests.
The prior Python app was removed only after committing [the gap inventory](REMAINING_WORK.md).
