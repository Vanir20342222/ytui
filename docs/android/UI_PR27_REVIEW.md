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

Merged as **`284ff617421e5bf596ef735791539b6eed7f71f0`** after qualifying head
`b35ac94848326ae85d6f709904ddef1117e8edab` against current main.

- [Android qualification](https://github.com/Vanir20342222/ytui/actions/runs/41071506514): **33 pass, one explicit public-network skip, zero failures**. Portrait/landscape search width, centering and chip heights pass. Existing keyboard/account navigation, all themes, player styles, offline/export, queue, About/tablet navigation and fullscreen checks pass. No app ANR or Rust panic markers.
- Strict actual window pixels: indexed **1645 ms**, catalog cold/prepared **979/780 ms**, HLS cold/prepared **935/711 ms**. No startup deadline changed.
- The tablet catalog and VU screenshots were inspected: the search input is capped and centered, leading artwork captions are intact, and the VU strip ends are inset without corner clipping. Evidence is in artifact **12985888129** of that Android run.
- [PR Rust](https://github.com/Vanir20342222/ytui/actions/runs/41071506757) and [push Rust](https://github.com/Vanir20342222/ytui/actions/runs/41071496143) pass: **366 workspace/extractor/media checks**, 31 explicit ignores, formatting and strict Clippy. Local **19 JVM tests**, instrumentation compilation and Android lint pass. Full-history scans and [PR secret scan](https://github.com/Vanir20342222/ytui/actions/runs/41071506504) pass without new exemptions.

GitHub's post-merge checks run separately; these are the qualified pre-merge
results. The original merge did not publish an APK. The subsequent
[mobile.9 release](MOBILE9_RELEASE.md) now packages these UI changes with the
existing permanent identity; mobile.8 retains its independently qualified
historical source and signature.
