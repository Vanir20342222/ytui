# Android media UI redesign (M9)

On 2026-10-28 the user confirmed the preview installs and requested removing
terminal styling from Android, with YouTube/Netflix as visual references. This
supersedes the earlier monospaced, rectangular mobile design requirement. The
Rust engine and native Compose/Media3 architecture remain.

## Design

- Sans-serif type, clear heading hierarchy, rounded surfaces, restrained color
  and fewer visible borders. Preserve all 13 shared palette choices and imports.
- Artwork-first adaptive video grids; square Music artwork and station cards.
  Move secondary card actions into a touch-accessible menu instead of displaying
  six buttons under every result. Preserve provider categories and continuations.
- Icon-and-label bottom navigation on phones, navigation rail on tablets; a
  compact artwork mini-player and a larger player with prominent transport.
  Only allocate a tablet player column when there is something to play.
- Rounded search, category chips, readable loading/error/empty states and
  grouped settings/account/download surfaces. All existing actions stay reachable.
- Native input, meaningful accessibility labels and minimum 48 dp action targets.
  Avoid adding artwork network requests for off-screen grid items or polling
  animations while paused/backgrounded.

## Acceptance

Build both ABIs, compile debug/release and instrumentation, run unit tests and
lint. Exercise actual typing, all shared themes, account navigation, compact and
wide layouts, card menus, downloads, queue and player controls in the emulator.
Capture populated offline fixture screens for visual inspection; they must never
replace live provider results in the application. Retain existing media/export/
encryption/solver regressions. Publish a tested development APK with a direct
download after runtime checks pass. Record actual results in VALIDATION.md;
physical device and real YouTube acceptance remain separate.
