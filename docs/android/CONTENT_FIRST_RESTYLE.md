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

Pending final native results. Baseline captures are retained before/; after/
will hold instrumented outputs. Local JVM/build/lint and native emulator outcomes
are tracked separately; fixture imagery does not qualify live YouTube or physical
hardware. Radio/Downloads only receive shared component styling, with captured
empty/existing state; station networking/export parity are outside this pass.
The missing original ZIP and lack of intermediate full-matrix runs mean those
specific brief requirements cannot be claimed. The final matrix is comprehensive
across the current implementation, rather than historical revisions.
