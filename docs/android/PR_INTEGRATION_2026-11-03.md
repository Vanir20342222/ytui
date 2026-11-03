# M9: integrate all open PRs and prepare Preview delivery

The owner requested resolving all conflicts and CI failures, merging all PR
features, and publishing a downloadable APK on 2026-11-03. This supersedes the
earlier PR-only/draft instructions in PRs #32, #47 and #48. Their historical
failed measurements remain in their evidence documents.

The combined source includes main's onboarding, account screen, watch-page
recommendations and desktop parity, plus:

- #32: shared media connections, parallel bounded byte prefetch, foreground
  priority, early activity-owned player-view preparation and intent cancellation.
- #47: separate copy-request/completion clocks and service cleanup waits that
  tolerate the expected preceding playback error.
- #48: SurfaceView rendering, Compose surface synchronization and full-window
  compositor evidence that includes opaque covers.

Conflicts were resolved by preserving the prepared-video lifecycle together
with recommendation/playlist context, `context_entry`/`context_skip`, resume and
manual-queue behavior. The duplicate video-probe readback constant was removed.
The strict completion deadlines and genuine output/playing requirements remain.

The legacy emulator TextureView renderer workaround forced codec recreation on
every surface change. SurfaceView permits normal codec reuse. The emulator still
initializes against the actual early-attached view: its placeholder buffers can
appear black. The emulator-only software decoder selection remains; physical
devices retain Media3's standard renderer and normal codec selection.

Network settings drafts now survive activity recreation with the open category.
The native input regression recreates the activity before checking the retained
text, without applying a proxy.

CI uses a 720×1600 display at 280 dpi, proportionally scaling Pixel 7's
1080×2400/420-dpi profile so logical layout dimensions remain unchanged. This
bounds SwiftShader and full-window compositor readback work on hosted four-core
runners. Tests still capture the complete composited window with covers/overlays
and charge completion against the unchanged deadline. Android's screenshot API
already copies to CPU internally, so a proposed subsequent GPU crop could not
reduce that work and was removed. Indexed observation starts after the source's
real decoded frame/playing state while the view-frame guard completes. Both
genuine RGB evidence and usable output must still finish under 1000 ms. The new
emulator profile is not comparable physical-device latency evidence or evidence
of an application speedup. Responsive tests retain their explicit size overrides.

## Checks and delivery

Local combined-source checks: 390 workspace Rust tests passed, 31 explicit
ignores; formatting and strict workspace Clippy passed. The 15 vendored mobile
playback tests, offline CI media setup checks, native solver vectors and 11
signing/publication helper tests passed. Android compilation/JVM/lint and hosted
emulator results are recorded below when completed.

First integrated source `812dccf`, workflow merge `f5d2e74`, Android run
[41610643278](https://github.com/Vanir20342222/ytui/actions/runs/41610643278):
APK/JVM/lint/inspection passed; three emulator cases failed. Cold usable output
was 630 ms but full-window evidence completed at 1226 ms; indexed usable output
was 979 ms with evidence at 1003 ms. The bright-white video contrast test also
failed actual-pixel validation, motivating retention of the emulator's real-
surface initialization guard. The catalog/settings shard passed. These failures
are retained; no threshold was relaxed.

`android-preview-*` release tags now publish only the exact APK from successful
Android/Rust/Secret scan runs for the tagged current-main commit. Publication
checks the source marker and inspection checksum, includes both ABIs/notices,
and retains the tested signature. It performs no rebuild or signing.

The default Preview version is `0.2.0-alpha.12-mobile.11`, code 11. The permanent
stable signing identity is outside this cloud workspace, so this delivery uses
`dev.ytui.android.preview` and a CI debug identity, separate from the stable app.
Do not uninstall an existing installation to work around a signing mismatch.

Physical Tab S9/Nothing Phone acceptance, public YouTube latency and account
behavior, audible output and stable-key update qualification remain open. M9
remains in progress; offline CI cannot close those gates.
