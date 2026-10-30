# Content-first Android restyle (M9)

The owner supplied `ytui-youtube-restyle-agent-prompt.md` and
`youtube-style-and-feel.md` on 2026-10-30 and requested implementation. Scope is
presentation only: preserve playback, Rust, provider/network/cache/account/data
contracts, navigation actions and all existing options. Keep ytui branding and
shared palettes. No YouTube logos, proprietary fonts or new service features.

## Baseline and decisions

The brief names `427842c`, superseded by mobile.8 playback and the merged PR27 UI.
Work starts at current main `05903aa`, preserving those changes. The screenshot
ZIP named in the brief was not supplied. Equivalent exact-source mobile.9
qualification captures (app source `553fe49`, unchanged app code at `05903aa`)
are retained at `/workspace/ytui-restyle-evidence/before`. This is retained prior
emulator evidence, not a fresh baseline run or a comparison against the missing
ZIP. Pixel 7 API35 suite and 2560x1600 tablet captures are the existing profiles.

The owner explicitly kept the YouTube label. The owner requested neutral dark as the default. Fresh mobile configuration now
selects ytui-dark; existing saved selections remain intact. Android maps that
palette's background/surface/text to central neutral tokens, keeping its teal
accent. Other shared palettes and desktop rendering remain unchanged. This
explicitly authorized default-only change is the sole Rust change; it does not
change extraction, networking, playback or data contracts. The current mini player exists and will be
restyled. MediaCard exposes creator and existing view/date metadata, but no
channel-avatar field: use a neutral creator initial, without network requests.
The old brief's video −5/+15 illustration differs from current configurable
Media3 controls. Preserve current seek behavior and amounts.

## Work groups

1. Shared spacing, shapes, type, neutral action and contrast roles.
2. P0: app bar/search, full-bleed feed, music covers, unified chip row, navigation.
3. P1: focused player/seek appearance, Media3 control layout, sheets, settings,
   account and dialogs. Preserve content descriptions and test tags.
4. P2: responsive grid and wide search rows; accessibility and screenshots.

Each group receives a commit. New visual assertions verify intended geometry;
existing behavior assertions and strict startup deadlines remain intact. Final
results, screenshot locations and unverified requirements are recorded below.

## Implementation and intended expectation changes

P0 changes are in `ComposeHost.kt` and `MediaDesign.kt`: slim branded app bar,
Browse icon retained, full-bleed 16:9 video art, creator-initial avatar and existing
metadata, single horizontal source/category row, square music art, inverted pills,
neutral mini-player and navigation without selection indicators. YouTube is not
renamed. Search keeps its submit action/content description; focus exposes Back
and Clear controls. The existing 720dp search cap remains; text inset changes
from 20dp to 16dp, so the geometry expectation changes by 8dp total.

P1 changes include `FocusedPlayer.kt`, `PlayerChrome.kt`, the native controller
layout and fullscreen presentation binding. Seek drag semantics/commit handlers,
seek amounts, playback/surface lifecycle, auto-hide and tap gestures are retained.
Progress uses a slim line and persistent round thumb. Media3's existing
captions/settings and collapse action move to the top, with gradient scrims and
translucent transport disc. There is no Cast button because no casting feature
exists. Classic, vinyl, orbit and VU remain. Card actions preserve all conditional
collection/live/station options. Card, queue, app speed/quality controls are
sheets; Settings is a full page; theme choices are a radio dialog; account loses
the tinted hero while retaining all privacy/sign-in warnings and import paths.
Native Media3's built-in track/speed popup remains owned by Media3: moving its
commands into another controller would exceed this visual-only pass.

P2 adds quarter-width thumbnails and shared action sheets for tablet search
results while keeping the same lazy-grid preparation/continuation behavior.
Large text can scroll player controls only when necessary to retain access;
normal single-song presentation stays bounded. Navigation permits wrapped labels.

Tests intentionally adapt only the text inset and Android neutral-dark expected
paint roles. Existing action labels, tags, native media behavior and strict
2000ms startup deadlines remain. Additional tests cover full-bleed art, 16:9,
source/category alignment, 48dp chips, dark-default preservation, imported-colour
contrast, 2x system text, wide search actions and 13 entries × light/dark system
mode × phone/tablet across feed/music/settings (156 captures). The 13 entries
include System, so repeated palette renders are expected.

## Validation

Local debug APK, instrumentation APK and lint pass; **20 JVM tests** and
**337 workspace tests** pass (31 explicit workspace ignores). The owner-approved
default/theme-preservation test passes. Final native outcomes, artifact downloads
and the screenshot review are published in [PR #29](https://github.com/Vanir20342222/ytui/pull/29);
its checks identify the current qualified revision. Baseline captures are retained
in before/, with native instrumented outputs in after/ in the review workspace.
Local checks, native emulation and device/live acceptance stay separate: fixture
imagery does not qualify live YouTube or physical hardware. Radio/Downloads only receive shared component styling, with captured
empty/existing state; station networking/export parity are outside this pass.
The missing original ZIP and lack of intermediate full-matrix runs mean those
specific brief requirements cannot be claimed. The final matrix is comprehensive
across the current implementation, rather than historical revisions.

### First native run retained (not the final qualification)

PR head `78835ec` / workflow merge source `27e6588`, Android run
[41105556355](https://github.com/Vanir20342222/ytui/actions/runs/41105556355):
**35 pass, one failure, one public-network skip**. The new wide-search geometry
case measured its resized image before IO decode completed; the final test now
waits for the actual image node before retaining the same quarter-width assertion.
Existing media/strict-startup cases pass. No app ANR or Rust-panic markers appear.

The run retained the full 156-image matrix. Visual inspection of all 12 contact
sheets identified asynchronous capture problems: some Appearance captures still
showed the preceding Settings frame. Review captures now await real UI idle;
playback deadlines are untouched. Review also corrected duplicated selected pills
for equivalent source/category requests, preserved a stable contrast-safe teal
brand across themes, left-aligned palette import, filled navigation icon details,
large-text navigation space and retained cover-play accessibility labels. A native
play/pause icon still contained its original solid disc; original white glyph
vectors replace that drawable inside the new translucent disc. These findings
are kept with the initial artifact rather than represented as final passes.

## Changed-file map

| Group | Files |
| --- | --- |
| Tokens/P0 | `MediaDesign.kt`, browsing/app-bar/search/cards/navigation/mini-player portions of `ComposeHost.kt` |
| P1 | `FocusedPlayer.kt`, `PlayerChrome.kt`, `FullscreenActivity.java` (presentation bindings only), panel/settings/account portions of `ComposeHost.kt`, `ytui_controller.xml`, `ytui_player.xml`, original glyph vectors `ytui_{collapse,play,pause}.xml`, collapse accessibility string |
| P2 | Wide search and accessibility geometry in `ComposeHost.kt`; `CatalogUiTest.kt`, `NativeUiTest.kt`, `MediaDesignTest.kt`, screenshot collection in `tools/android/run-device-tests.sh` |
| Explicit owner exception | Fresh-config theme default and preservation test in `crates/ytui-mobile/src/lib.rs`; no extraction/network/cache/media behavior changes |

`ytui_controller.xml` adapts the existing Apache-licensed Media3 control layout;
its copyright/license header is retained. Glyphs are original simple vectors.
No font, asset service, tracking endpoint or dependency was added. Stable signing
identity, account/session files, library state and release assets are untouched.

Review also bounds sheets to their content up to a 90% window limit, instead of
forcing short menus to occupy nearly the entire screen. Full-page
settings and the focused player retain their bounded full-screen layout.


### Qualified app implementation and screenshot-harness follow-up

Application implementation `debbbd1` / workflow merge `5fc2615` passes Android
[41108974475](https://github.com/Vanir20342222/ytui/actions/runs/41108974475):
**36 native passes, zero failures, one public skip**. Rust
[41108974473](https://github.com/Vanir20342222/ytui/actions/runs/41108974473) passes
**367 workspace/extractor/solver/media cases**, 31 explicit ignores; JVM 20,
lint, both ABIs, APK inspection and secret scans pass. App ANR/Rust-panic markers:
zero. Actual window-pixel confirmations stay within the unchanged 2000ms budget:
progressive cold/prepared **849/1053ms**, indexed **1344ms**, HLS **941/794ms**.

All 156 matrix images are present with exact phone/tablet dimensions. Review of
all contact sheets confirms neutral chrome, teal brand, consistent selection and
settings. A few supplemental native panel captures still precede their window
transition, and a tablet-search capture catches resized artwork loading. The
follow-up changes only screenshot tests: wait for real UI idleness for native
panels and actual decoded cover pixels before review captures. Application,
networking, image cache and playback code remain identical to the qualified app.
The PR's current checks/report identify final screenshot-harness qualification.
