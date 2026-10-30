# M9: UI PR #27 review and integration

[PR #27](https://github.com/Vanir20342222/ytui/pull/27) adjusts VU meter clipping
and placement, artwork crop alignment, common chip sizing, wide search-field
width and About text. Its original source `f9041e9` predates the indexed playback
work. Main is merged into its branch, preserving history and the mobile.8
playback fixes accepted on the owner's Tab S9.

## Review findings

- The proposed `fillMaxWidth().widthIn(max = 720.dp)` search modifier cannot
  shrink the exact-width constraint imposed by `fillMaxWidth`. Corrected to
  `widthIn(max = 720.dp).fillMaxWidth()` with center alignment. The narrow input
  still fills its available width.
- The VU strip's inset span could become negative in a very narrow container.
  Clamp it to zero before calculating segment positions/widths.
- Retain the shared 48dp FilterChip for source/category/library rows, the
  leading-edge artwork crop and the regrouped inset VU strip. Retain complete
  About copy and its existing scrollable panel.

A native geometry regression checks actual portrait and landscape search width
and centering, plus consistent minimum-48dp source/library chips. The existing
actual artwork/player-style, all-theme, tablet menu/About, fullscreen and strict
startup checks remain required. No startup deadline is weakened.

The original [Android run](https://github.com/Vanir20342222/ytui/actions/runs/41060137391)
failed its HLS startup check at 2193 ms, with the remaining 28 cases completing
(one public skip). This is retained as failed evidence, not waived on the basis
of a successful compile. Fresh qualification against current main is required.

The owner's six mobile.8 Best-quality attempts now pass the observed two-second
video target; [the physical record](MOBILE8_INDEXED_STARTUP.md) contains all six
results and preserves the earlier failures. This does not complete broader M9.

## Validation

Local Android checks and fresh PR checks are in progress. Native screenshot and
test results must be inspected before merge. This change does not publish a new
stable APK; the existing mobile.8 release retains its independently qualified
source and signature.
