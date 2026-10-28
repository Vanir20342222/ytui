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

## Implementation checkpoint (not yet an APK qualification)

- `aaf12c5` removes all app-wide snackbar confirmations and drops platform
  `notice` events. Errors belong in normal screen content, not timed overlays.
- Android reuses account/network/quality-scoped native extractor clients and
  five-minute-or-shorter stream leases. Signed expiry margins, cancellation,
  deduplication and cache memory budgets are checked offline. The vendored
  extractor reuses bounded public versioned player JavaScript assets; independent
  versions do not share a blocking network lock.
- Visible finite-playlist items seed Next while the complete collection loads.
  Accepted decoder starts still commit queue/history; warmed shuffle candidates
  remain stable and manual queue entries have priority. Related media are the
  default tail outside collections. Streaming bytes use a separate bounded LRU;
  speculative failures do not interrupt the current player.
- A focused, non-scrolling song player exposes advanced controls in a secondary
  panel; native video overlay controls and landscape fullscreen share the same
  decoder/session. Optimistic seeking reconciles actual Media3 positions.
  Category/tab swipes, downward player minimization, double-tap video seeking,
  reduced-motion-aware transitions and icon close controls are implemented.
- Settings now have Appearance, Playback, Offline & storage, Video information,
  Network, Privacy and Account categories. Personal playlists have direct
  YouTube/Music source chips and a Play all action. Feed counts/dates are kept
  without another request and extraction enriches the playing media metadata.
- Offline/Playlists/Transfers views, Rust-owned local playlist edits and
  Android-granted Music/Video document trees are implemented. Completed compatible
  progressive media copies/muxes through the existing export adapter. Files have
  a private local URI index; missing grants preserve cached playback. Audio/video
  modes and video qualities have separate download identities. Legacy URL-only
  completed downloads remain discoverable. Progressive format cache keys survive
  signed-URL renewal, while different formats/content revisions stay separate.
- Radio uses provider artwork first and curated sized genre photographs as a
  fallback; [sources/license](RADIO_ARTWORK.md) are recorded. Vinyl/VU/Orbit use
  cached cover art and pause-aware animation/PCM, without generated online demo feeds.
- Private local signing with SSH-encrypted recovery replaced GitHub secret setup;
  [recovery and the stable channel](SIGNING.md)
  preserve both old installations. No signing key is committed or published.

Current evidence: local debug/release compile/unit/lint and strict Rust Clippy
pass at their recorded checkpoints. The newest Rust/mobile playlist, expiry,
shuffle, metadata and cache checks pass. Actual new fullscreen, blocked-playlist
Next, folder-picker/export/local EOF and populated native UI cases are authored;
CI device execution, final exact-source checks and updated screenshots remain
required. Three-second real YouTube starts and physical-device acceptance are
not established by these fixture or build results.

The next hosted pass (`f847568`, run 40759812773) passes the real folder grant,
legacy/new exports, cache-free local playlist EOF and all 13 theme checks. Its
only remaining failure is a diagnostic selecting one Compose root while two
windows exist. The player screenshot confirms bounded controls/artwork geometry;
the diagnostic is scoped to the player and cover-ready/visualizer/fullscreen
render assertions are being tightened before final qualification. A local stable
APK has passed a full temporary transfer, checksum, certificate, package and
source-marker verification. Neither that intermediate APK nor a failed device
checkpoint is a public release.

The stronger `67863cf` render pass found a real fullscreen defect: registering
the exit listener before setting its initial state immediately finished the
activity. Listener ordering/state handling is repaired, and first-frame timing
is retained across surface switches. The Vinyl assertion now permits its two
intentional cover instances. Local/indexed media also explicitly block related
recommendations outside offline test mode; a stopped-worker regression prevents
even a failing guard from sending network requests. Transfer URLs are masked
before workflow environment logging, and recovery files have Git ignore guards.
These repairs require a new device pass; the failed screenshot/checkpoint is
retained as evidence rather than treated as a release qualification.

Public-network attempt: [Android 40781371045](https://github.com/Vanir20342222/ytui/actions/runs/40781371045)
explicitly tried a public YouTube video on `67863cf`. Native extraction failed
with invalid player responses and the extractor's likely-IP-block diagnostic;
no real YouTube playback or three-second service latency was established. This
run also contains the two older player/assertion failures described above.
Neither a skipped default live case nor local decoding fixtures erase that
separate failed network acceptance result. Do not retry aggressively or import
personal credentials into hosted CI to make it pass.
