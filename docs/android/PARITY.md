# Android / desktop parity and remaining work (M9)

Implementation does not establish live-service acceptance. Android is a preview;
Nothing Phone (3a) hardware has not been available in this workspace. The user
tested the earlier preview on a Galaxy Tab S9 and reported failed text input,
theme application and account navigation; these require native UI re-acceptance. The desktop app and its existing private state remain separate.

| Area | Android implementation | Qualification / remaining work |
| --- | --- | --- |
| Application UI | Native Compose grids, keyboard/account/settings, phone navigation, tablet rail, bounded focused player and gestures | 28 actual emulator cases pass, including all 13 themes and disabled-gesture behavior; physical cutout/accessibility/jank acceptance remains |
| Themes | 13 theme entries including system; shared dark/light palettes and document import | System light/dark maps to ytui dark/light; GNOME/Omarchy/Caelestia services are Linux-only, import their files instead |
| Browsing | Shared Rust YouTube/Music feeds, search, categories, shelves, continuations, bounded transient retries | Personalized/anonymous live feeds must pass on devices/network; no generated catalog substituted for provider failure |
| Images | Sized bounded disk/memory cache, virtualized rows, cancellable transient retries and provider/genre radio art | Real loopback HTTP/decode/render and first-503 recovery pass; cold images need their first fetch, device memory/jank checks remain |
| Native extraction | Pinned Rust extractor; Android isolated offline EJS batch, same strict output checks | Android System WebView feature support, real public/native corpus, account/429/consent acceptance required; no Python fallback on mobile |
| Playback | Media3 with direct per-surface event forwarding, native transports, landscape fullscreen, captions/tracks/speed and quality via re-resolution | Actual inline/controls/fullscreen/return pixels and native pause/Next pass; live YouTube, audible output, hardware/HDR/power and OEM interruptions remain open |
| Music/radio | Native channel/station providers, queue advancement, ICY song titles retaining station identity, vinyl and PCM-reactive VU/Orbit, persistent player volume | Real stations/ICY metadata, actual sound, battery and long-running behavior need qualification; external desktop cliamp binary cannot be launched on Android |
| Library | Durable local saved items, editable offline playlists, queue/resume and history committed on accepted decoder starts | Actual playlist edits, granted folders, cache-free local playback and offline EOF pass; desktop-to-mobile private library transfer remains open |
| Queue | FIFO/shuffle, decoder-playing commit, repeat-one, button reordering, finite collections, related-media tail and scoped Next preloading | Manual queue priority and blocked-tail adapter Next pass; process-death derived context rebuild and drag reorder remain limitations |
| Downloads | Media3 durable cache, complete collection preparation, mode/quality identities, Offline/Playlists/Transfers views, separate Music/Video folders and compatible automatic exports | MP4/M4A/WebM copy or compatible mux is tested. Segmented/HLS exports, unsupported codec combinations, MP3/FLAC transcoding, cuts/chapters, scheduling and desktop output templates remain unsupported |
| Playlist preparation | Bounded collection expansion, checkpoint file and per-entry progress, cooldown, download-index acknowledgement before advancing | Cancel/remove preparation is supported; retry currently creates a new preparation job (URL-keyed downloads deduplicate), failed-entry-only retry and retention cleanup need work |
| Account | Explicit sign-in page with Use this session / Cancel controls, cookie document/header import; AES-GCM session envelope with a non-exportable Android Keystore key; private atomic persistence, legacy migration, reload and disconnect | Sign-in may be rejected by Google; Android cannot import another app's private browser DB; no OAuth claim. Missing/invalidated keys require reconnecting; hardware-backed key availability depends on the device. Synthetic encryption/tamper/migration/disconnect cases pass on the emulator; real account and hardware-backed key acceptance remain |
| SponsorBlock | Shared public-ID API, marks, automatic skipping and replay, settings | Live timing, seeks and replay need media/device acceptance; it does not block YouTube UI/network ads globally |
| Lyrics | Explicit reviewed LRCLIB lookup, candidate selection, timed follow/seek, LRC/text document import, text editor, persistent per-song timing adjustment | Local lyric library is bounded to 100 songs / 8 MiB; edits are app-private, external sidecar export is not implemented; live LRCLIB/device text-input acceptance remains |
| Settings | Native Appearance, Playback, Offline & storage, Video information, Network, Privacy and Account categories; live shared palettes and personal playlist sources | Native entry/input/theme paths pass; desktop-specific tools are excluded and advanced desktop profile/form parity remains open |
| Background | MediaSession foreground playback, download service, WorkManager preparation | OS power limits/Android 15 service limits and OEM battery management apply; real lockscreen/headset/Bluetooth/service recreation gates remain |
| Linux integration | Desktop continues using mpv, MPRIS, browser import and desktop themes | MPRIS, system keyring, NetworkManager, Proton control, external cliamp and executable launch are intentionally desktop-only |
| Distribution | [Signed mobile.9](https://github.com/Vanir20342222/ytui/releases/tag/android-0.2.0-alpha.12-mobile.9), stable identity, universal ARM64/x86-64, signature/source/16 KiB checks and private SSH-encrypted recovery | Public APK hash verified; no GitHub signing secrets required. Old Preview apps remain separate. Physical in-place stable updates remain to be accepted |

Before promoting this preview, finish outstanding mobile capability gaps, qualify
all three target usage modes (phone, tablet, desktop), retain the stable identity and its encrypted recovery, qualify physical updates,
and retain the existing desktop release gates. Do not mark M9
complete based on compilation or simulated UI frames.
