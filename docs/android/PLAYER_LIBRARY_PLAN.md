# Android player, latency and offline library follow-up (M9)

The user accepted the modern UI, then reported slow starts/Next/seeking and
requested a focused player, fullscreen, transitions/gestures, simpler copy,
visualizers, station artwork, richer settings, personal playlists and offline
libraries/folders. The user explicitly chose related-media preloading by default
and separate Android folder-picker Music/Video destinations for completed media.

## Implementation order

1. Trace and repair Next candidate/context handling; measure extraction, first
   decoded frame and Next separately. Add bounded account/network/quality-scoped
   resolution and next-item prefetch, expiring signed URLs before reuse. Warm
   selected next media without committing queue/history until actual playback.
   Tune Media3 startup buffering and download concurrency. A three-second cold
   start is a goal, not a guarantee independent of YouTube/network conditions.
2. Make seeking respond immediately while dragging and reconcile acknowledged
   positions. Build a dedicated non-scrolling Music/Radio player and YouTube-like
   video surface with overlay controls, true landscape fullscreen/system-bar
   handling, swipe-to-minimize and double-tap seek. Preserve accessibility,
   track/caption/quality/SponsorBlock/queue controls through secondary sheets.
3. Add reduced-motion-aware transitions, tab/category gestures and icon-based
   close controls. Remove decorative subtitles/marketing copy. Refine vinyl,
   VU and Orbit layouts and active/paused/background PCM rendering. Use original
   programmatic visuals unless image assets materially improve the result.
4. Group settings into Appearance, Playback, Offline, Network, Metadata and
   Account. Add configurable upload/date/views/channel/duration/description
   visibility and fetch metadata without blocking playback. Expose personal
   YouTube and Music playlists directly and retain account-required recovery.
5. Add an offline library and durable local playlists. Keep progressive/cache
   downloads resumable; use granted Android document-tree destinations for
   completed compatible audio/video, indexed for local playback. Preserve cache
   access if a folder is unavailable and surface export/storage failures. Never
   delete unrelated user documents or silently request broad storage access.
6. Supply station artwork from provider-owned icons or documented public image
   sources, cache bounded image sizes, and keep attribution/provenance. Cover
   failure must not prevent listening.

## Acceptance

- Rust/Android unit regressions for candidate selection, expiry/isolation,
  prefetch races, finite seek/progress, storage grants and local playlist edits.
- Actual Media3 fixture timings for cold local starts and prepared Next; rapid
  Next presses, manual queue priority, EOF, pause/seek, account/network changes,
  offline transitions and cancellation must remain correct.
- Native phone/tablet tests for non-scrolling players, fullscreen/rotation/back,
  gestures/reduced motion, all shared themes, categorized settings, personal
  playlist entry points, offline library/folder flows and real visible video.
- Build both ABIs, debug/release compile/unit/lint, strict Rust checks, APK
  signing/alignment/notices, secret scan and actual screenshots. Publish the
  exact qualified development APK with a direct download.
- Record actual timings and public YouTube/device limitations honestly. Preserve
  prior APK/data and all existing desktop/service validation gates.
