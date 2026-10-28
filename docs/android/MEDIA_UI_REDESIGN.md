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

## Development installation

The previous installed preview and the redesign's first CI APK have different
public signing certificates. An in-place update is therefore rejected by
Android. Debug builds now use a separate `dev.ytui.android.preview` identity and
**ytui Preview** launcher, preserving the older app and its private data. Normal
release identity is unchanged. Future updates still require matching signing
keys; see SIGNING.md for the owner-supplied stable identity.

## Acceptance checkpoint

Source `5f34083` passes Android/Rust/Secret scan workflows: 317 offline Rust tests,
14 Android unit tests (debug/release locally) and 20 emulator cases, with one
public YouTube case explicitly skipped. Both ABIs, native input/all themes, actual
artwork, tablet layout, transport and existing media/export/account regressions
pass. [Validation](VALIDATION.md) records exact APK/hash and earlier failures;
[actual screenshots](../screenshots/android/README.md) show the result. Physical
device/live and stable owner signing gates remain open.
