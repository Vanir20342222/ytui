# Open Android issues after the 2026-11-05 review fixes

Not everything from the review is fixed. This file lists what remains: review items not
closed, follow-ups the fix agents reported, and missing features. Source list:
[ANDROID_REVIEW_2026-11-05.md](ANDROID_REVIEW_2026-11-05.md). Merged in PR #109 (squash `9ac0c44`).

**Evidence level for every fix below: fixture/unit only.** Nothing was run on an emulator, a
physical device, or against live YouTube. Treat "fixed" as "code changed and unit-tested".

## 1. Review items not fully closed

| ID | Status | What is still open |
| --- | --- | --- |
| AR-19 | Deferred | Exported files keep their internal cache copy, so saved media uses twice the space. Removing the copy changes offline playback; needs an owner decision. |
| AR-24 | Analysed, not fixed | Cause is a timing flake: the test advances the clock by hand, so a closed sleep sheet can still be composed. The test also has no `finally` to stop its 30 s screen recording, which likely made the next test on the same shard time out. Proper fix needs a longer wait or frame-stepping in the test, which the PR rules forbid without owner approval. |
| AR-13 | Partly fixed | SponsorBlock now applies to downloaded YouTube videos. Offline captions are still dropped (see AF-13). |
| AR-15 | Partly fixed | Scheduler added for Android 8–11. On Android 12+ Media3 already waits for the network, but if the OS kills the app while a download waits, it resumes only when the app or a new download starts the service. |
| AR-16 | Partly fixed | Expired-link refresh and space check are implemented but untested on a device. The space check looks at one download at a time and does not reserve space for other queued downloads. |
| AR-17 | Partly fixed | Retrying a playlist job in place has no automated test; WorkManager is not exercised in JVM tests. |
| AR-22 | Partly fixed | The cleartext rule covers Java-side HTTP (OkHttp, HttpURLConnection) only. The Rust engine's own HTTP client is not covered. |
| AR-08 | Partly fixed | Links are not verified App Links; Android 12+ opens them in Vidcue only if the user enables that in "Open by default". A share not yet handled is lost if the OS kills the process first. |
| AR-10 | Partly fixed | Android may ignore the requested orientation on large screens and in multi-window. |
| AR-01 | Fixed in code, not reproduced | A sandbox crash was not reproduced. A connection that never completes is still waited on until each call's deadline; it is not replaced. |
| AR-21 | Fixed, with leftovers | Some dead Java fields remain: `MediaBridge.texture` is always null but still read by `PlaybackService`'s resize branch; `videoFrames` is kept for tests. |

## 2. New issues found by the fix agents

- The volume slider in `ComposeHost.kt` (~line 2290) still falls back to 0.7 for display when the status has no `volume` field. The service always sends one, so this is latent.
- `CatalogUiTest` screen-recording cleanup (see AR-24) is a test-harness bug that can break unrelated tests.
- Two `CatalogUiTest` sleep-sheet checks changed to match the new "Cancel timer only while a timer runs" behaviour. They were compiled but not run.
- The playback sleep mode "End of this track" replays the same item when Play is pressed after the pause, rather than advancing. Intended, but worth confirming with the owner.

## 3. Process gaps

- PR #109 was merged before CI ran on it. CI on `main` for that commit has not been checked in this session.
- The Rust workspace test run on the merged branch passed (450 passed, 0 failed, 31 ignored), but the 17 egui-only tests removed with the old UI are no longer counted.
- PR #109 bundles five fix groups in one PR because the session could push only one branch. Review-rule compliance (one topic per PR) is not met.
- Open PRs #78 and #48 overlap with download and playback files changed in #109. Conflicts are likely.
- Local Robolectric runtime jars were fetched manually from Google's Maven mirror because Maven Central rate-limited the build. These are environment-only, not repo changes.

## 4. Missing features (not implemented)

| ID | Feature | Notes |
| --- | --- | --- |
| AF-01 | Picture-in-picture | No `supportsPictureInPicture` or PiP entry. |
| AF-02 | Auto-fullscreen on rotate | Fullscreen only from the button. |
| AF-03 | Chapters, comments, clickable timestamps and links in descriptions | Not on Android. |
| AF-04 | Repeat whole queue | Only off and repeat-one; the repeat-all icon is unused. |
| AF-05 | Item actions: Play next, Go to channel, Share / Copy link, Start radio, Remove from history | Card sheet has Play, Queue, Download, Preview, Play all, Bookmark, Browse tracks. |
| AF-06 | Channel pages, subscribe, like | Read-only account; mutations are intentionally absent on desktop too. |
| AF-07 | Android Auto and Wear browsing | Only `MediaSessionService`; no `MediaLibraryService`. |
| AF-08 | Home-screen widget | None. |
| AF-09 | Localization | Android strings are hard-coded English; desktop has German. |
| AF-10 | Mobile-data download quality and a storage cap | Wi-Fi-only and a per-download space check were added (AR-16). No storage cap or reserved space. |
| AF-11 | Failed-entry-only playlist retry | Open PR #78 covers part of it. |
| AF-12 | Remote YouTube watch history | Local history only. |
| AF-13 | Offline captions | Downloads drop the caption catalog. |

## 5. Documented gates still open (from `docs/android/PARITY.md`)

- Live YouTube playback and the two-second startup target on a physical Tab S9.
- Audible output, OEM battery limits, and in-place update on a physical device.
- Drag-to-reorder in the queue; queue rebuild after process death.
- HLS, MP3 and FLAC export.

## 6. Not verified

- No emulator, device or live-account run covers any change in PR #109.
- The Android 15 dataSync timeout handling and the network security config are untested at runtime.
- Notification tap and progress, Wi-Fi-only switching, and the low-space refusal are untested on hardware.
