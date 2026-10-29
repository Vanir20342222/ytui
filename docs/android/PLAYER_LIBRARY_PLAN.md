# Android player, latency and offline library follow-up (M9)

Current source `cf8046508b96962f1b5ba9efc3529c4427d64b9f` is published as **mobile.4** after main Android/Rust/Secret checks and private signing. **24 runtime cases pass, one public opt-in case skips**. [Final qualification and remaining gates](PLAYER_LIBRARY_VALIDATION_2026-10-29.md) supersede the historical intermediate checkpoints below. Real YouTube latency and physical-device acceptance remain open.

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

`5ba553f` passes populated covers, all three visualizers, folder/local-only
guards and decoded fullscreen pixels. The remaining failure is fullscreen Back
navigation; an explicit AndroidX Back callback and actual exit-button/reopen/Back
checks repair that final transition. No failed artifact is published as qualified.
Manual capture review also caught a doubled vertical offset on the vinyl label;
top-origin alignment and a measured hub-position assertion now keep it centred
on the spinning record. Fullscreen navigation checks wait for the resumed window
to own input focus, so Back cannot accidentally target the previous dialog.
The next exit-button check exposed a missing fullscreen control after a surface
tap. Surface single taps now explicitly invoke the native player's click
handling; double taps retain seeking and child controls keep their own events.
The test accepts both native full/minimal toolbar layouts and retains a control
screenshot/accessibility hierarchy before asserting, so remaining failures have
concrete evidence.
The retained `db1c47e` hierarchy identifies the remaining blocker as Android's
first-use immersive-mode tutorial (`android:id/immersive_cling_title`). The
surface/control diagnosis above was too broad: this system overlay intercepted
the test taps. Instrumentation now acknowledges that specific OS prompt before
checking the native toolbar and exit routes; no application or permission dialog
is blindly dismissed, and production does not suppress the system tutorial.
The first acknowledgment pass then exposed a harness ordering error: checking
app input focus before dismissing the system hint prevented acknowledgment from
ever running. Entry/re-entry now acknowledges the OS hint before waiting for
app input focus. The retained screenshot and `ImmersiveModeConfirmation` window
log establish this specific cause; no further production change was needed.
The next capture confirms the tutorial is gone. Its timestamps show UIAutomator
waiting 5.7 seconds for global idleness while the native toolbar auto-hides after
5 seconds; native-control queries now use a scoped 100 ms idle interval, restored
after the test. A separate rare Compose test-frame continuation constructed a
dialog on a Default worker. Snapshot application explicitly returns to Main,
and Compose test effects use Main too. Native video controls now respect both
system animation disabling and the app's reduced-motion preference.
`05f7f67` passes the UI-thread repair but catches an inline TextureView displaying
stale background pixels while the software decoder continues submitting frames.
Video now uses Media3's recommended SurfaceView path with its AndroidView/Compose
surface-sync workaround. The same visible RGB checks remain; failure evidence
also collects SurfaceView PixelCopy output. [Media3 surface guidance](https://developer.android.com/media/media3/ui/surface)
describes SurfaceView's frame-timing, power and HDR advantages; no device timing
claim is inferred from that guidance.

The `35c7616` device run still fails visible inline video (SurfaceView PixelCopy
is black despite continued decoding) and the tablet catalog's decoded-cover
wait. The focused player now stays in the activity window instead of a separate
dialog window. Artwork retries transient failures at most twice with bounded
backoff and cancellation checks; the catalog case deliberately fails its first
cover request to exercise recovery. Failure evidence includes the catalog state
and decoder-versus-displayed-surface ownership. Local debug build, 14 JVM tests,
lint and instrumentation compilation pass; hosted pixel and tablet checks are
still required before this change is qualified.

`a4df91f` passes visible inline and landscape video, the tablet catalog, first-503
artwork recovery and all 13 themes (25 total cases: 23 pass, one failure and one
public opt-in skip). The remaining fullscreen-exit assertion
is intercepted by Android's delayed first-use tutorial: the retained hierarchy
and screenshot show `android:id/immersive_cling_title` arriving after the initial
three-second observation. Control acquisition now observes that specific system
prompt throughout the touch interaction, then verifies both exit paths and
visible video after returning. Music-only activity transitions respect reduced
motion, video surfaces never animate between windows, and the covered catalog
is removed from accessibility while the focused screen is open. These stronger
checks need a new emulator pass before final publication.

`3cd4028` passes all 24 default emulator cases (25 JUnit cases, one explicit
public skip), Rust and Secret scan. There are no app ANRs or Rust panic markers.
Inline/fullscreen/return RGB frames are visible. Manual capture review still
finds a black frame behind the native fullscreen toolbar, which the separate
frame/control assertions did not cover. Publication remains blocked while the
test requires decoded colours and visible native controls in the same frame.
The player now uses TextureView for consistent UI composition; SurfaceView's
power/HDR advantages do not outweigh a failed visible-control presentation.
Hardware/HDR/power acceptance remains a separate device check. The focused
header also respects the gesture setting, has a pane announcement, and a native
touch regression verifies that disabling gestures preserves icon minimization.

`5d84e14` passes the disabled-gesture check, but TextureView alone still fails
initial inline RGB rendering. The attached view and raw decoder both report
996×560 while the texture bitmap is black. The native view now binds through a
direct `ForwardingPlayer` adapter instead of the session's cached
`ForwardingSimpleBasePlayer` state. Per-surface frame events reach the view
directly; Next/Previous and user-pause/reconnect intent still route through ytui.
The session keeps its separate live-metadata/controller adapter. The runtime
case also exercises the real native play/pause button, and the blocked-playlist
Next case uses the presentation adapter. Qualification is pending; this does not
claim that decoding telemetry alone proves visible rendering.
