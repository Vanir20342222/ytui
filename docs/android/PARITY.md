# Android / desktop parity and remaining work (M9)

Implementation does not establish live-service acceptance. Android is a preview;
Nothing Phone (3a) hardware has not been available in this workspace. The user
tested the earlier preview on a Galaxy Tab S9 and reported failed text input,
theme application and account navigation; these require native UI re-acceptance. The desktop app and its existing private state remain separate.

| Area | Android implementation | Qualification / remaining work |
| --- | --- | --- |
| Application UI | Native Jetpack Compose controls/IME, phone bottom tabs and tablet split pane; bounded Back history | Native input/theme/account emulator regressions added; results and physical Tab S9 re-test pending. Rotation, cutouts, gestures and accessibility review remain required |
| Themes | 13 theme entries including system; shared dark/light palettes and document import | System light/dark maps to ytui dark/light; GNOME/Omarchy/Caelestia services are Linux-only, import their files instead |
| Browsing | Shared Rust YouTube/Music feeds, search, categories, shelves, continuations, bounded transient retries | Personalized/anonymous live feeds must pass on devices/network; no generated catalog substituted for provider failure |
| Images | Sized bounded disk/memory cache, virtualized rows and bounded async decode | Cold-cache images necessarily need a first fetch; device rendering/jank/memory measurements required |
| Native extraction | Pinned Rust extractor; Android isolated offline EJS batch, same strict output checks | Android System WebView feature support, real public/native corpus, account/429/consent acceptance required; no Python fallback on mobile |
| Playback | Media3 hardware decoder and native PlayerView/captions, split audio/video, seek, pause, volume, speed, tracks, quality by re-resolution | Device codecs, live reconnect, background/notification controls, interruptions and audible output require hardware/live checks; no claim of full YouTube player parity |
| Music/radio | Native channel/station providers, queue advancement, ICY song titles retaining station identity, vinyl and PCM-reactive VU/Orbit, persistent player volume | Real stations/ICY metadata, actual sound, battery and long-running behavior need qualification; external desktop cliamp binary cannot be launched on Android |
| Library | Private durable saved items, queue, resume and recently played/previous/forward history; writes follow accepted decoder starts | Desktop-to-mobile library transfer and rich local collection editing are future work |
| Queue | FIFO/shuffle, decoder-playing commit, repeat-one, touch reordering, automatic finite-collection continuation; manual items take priority | Derived collection context is rebuilt by selecting a collection after process death; related-recommendation autoplay and drag reorder gestures are not implemented |
| Downloads | Media3 durable private cache, paired video/audio, play offline, pause/remove/refresh failed stream; full-playlist preparation; completed progressive media export through Android's document picker | Export copies MP4/M4A/WebM or muxes compatible paired tracks without re-encoding; segmented/HLS exports and unsupported codec combinations are rejected explicitly. No MP3/FLAC transcoding, cut/split/chapter edits, scheduling or desktop output templates yet; cached files remain private until explicitly exported |
| Playlist preparation | Bounded collection expansion, checkpoint file and per-entry progress, cooldown, download-index acknowledgement before advancing | Cancel/remove preparation is supported; retry currently creates a new preparation job (URL-keyed downloads deduplicate), failed-entry-only retry and retention cleanup need work |
| Account | Explicit sign-in page with Use this session / Cancel controls, cookie document/header import; AES-GCM session envelope with a non-exportable Android Keystore key; private atomic persistence, legacy migration, reload and disconnect | Sign-in may be rejected by Google; Android cannot import another app's private browser DB; no OAuth claim. Missing/invalidated keys require reconnecting; hardware-backed key availability depends on the device. Device encryption/migration tests must pass before qualification |
| SponsorBlock | Shared public-ID API, marks, automatic skipping and replay, settings | Live timing, seeks and replay need media/device acceptance; it does not block YouTube UI/network ads globally |
| Lyrics | Explicit reviewed LRCLIB lookup, candidate selection, timed follow/seek, LRC/text document import, text editor, persistent per-song timing adjustment | Local lyric library is bounded to 100 songs / 8 MiB; edits are app-private, external sidecar export is not implemented; live LRCLIB/device text-input acceptance remains |
| Settings | Native appearance/player/network/account controls with shared Rust validated config; live palette application | Desktop-specific backend/tool paths are not offered as mobile capabilities; complete advanced desktop forms/profile parity remains open |
| Background | MediaSession foreground playback, download service, WorkManager preparation | OS power limits/Android 15 service limits and OEM battery management apply; real lockscreen/headset/Bluetooth/service recreation gates remain |
| Linux integration | Desktop continues using mpv, MPRIS, browser import and desktop themes | MPRIS, system keyring, NetworkManager, Proton control, external cliamp and executable launch are intentionally desktop-only |
| Distribution | Universal ARM64/x86-64 development APKs; main-only signed-build workflow with explicit versions and private owner credentials; signature/16 KiB checks | Disposable local signed build passes; the owner's stable identity and hosted signing environment still need setup, followed by device update acceptance |

Before promoting this preview, finish outstanding mobile capability gaps, qualify
all three target usage modes (phone, tablet, desktop), configure a stable release
signing identity and retain the existing desktop release gates. Do not mark M9
complete based on compilation or simulated UI frames.
