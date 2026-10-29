# Android and monorepo implementation plan (M9)

The user requested a Rust Android app with touch-first phone/tablet layouts,
terminal styling, existing themes/features, performance and actual build checks.
The desktop application must continue to work and keep its existing state.

The user authorized a native UI stack change on 2026-10-28 after failed physical
Tab S9 input, theme and account navigation. The engine remains Rust. See
[NATIVE_UI_MIGRATION.md](NATIVE_UI_MIGRATION.md) for the decision and acceptance
gates; earlier egui render tests do not qualify the new UI.

The user subsequently rejected terminal styling on mobile and requested a
YouTube/Netflix-inspired presentation. [The media UI redesign](MEDIA_UI_REDESIGN.md)
records the current visual requirements and acceptance checks.

## Architecture

- `apps/desktop`: existing Rust terminal application, with unchanged binary names.
- `apps/android`: native Jetpack Compose adaptive touch UI, Media3 video/player
  surface, and Android media/storage/service adapters.
- `crates/ytui-core`: shared models, queue/config/history, themes and policy.
- `crates/ytui-providers`: shared Rust YouTube/Music browsing, radio/cliamp,
  bounded HTTP/cache, session scoping, images, lyrics and SponsorBlock.
- `crates/ytui-mobile`: Rust Android engine/JNI boundary and retained legacy UI
  prototype; no desktop process,
  keyring, MPRIS, NetworkManager or browser-database dependency.
- Desktop services re-export shared provider APIs rather than maintain copies.

Native Android platform APIs replace desktop integrations: Media3 for decoding,
video/track/subtitle/speed controls, MediaSession/foreground playback; system
file picker/share intents for imports/exports; app-private state; Android system
light/dark following. Omarchy/Caelestia/GNOME palette files can be imported where
available, but their Linux desktop services do not run on Android.

Android cannot read another browser's private cookies without privileged access.
Use explicit app sign-in/cookie import with app-private storage and disconnect.
Native extraction retains the pinned Rust backend and its service/challenge
limitations; do not claim mobile extraction is fixed by a successful APK build.
No Python subprocess or downloaded executable is part of the Android app.

## Work and checks

1. Preserve desktop behavior while extracting shared portable code and themes.
2. Build phone/tablet navigation, feed/search/continuation, bounded thumbnails,
   theme/settings, account, queue/resume, downloads and playback flows.
3. Integrate Android media service, storage, permissions, notifications and
   player presentations; document unsupported desktop-only actions explicitly.
4. Add Rust/JNI-contract, HTTP, Android unit/instrumentation and lifecycle checks.
5. Build ARM64 and emulator APKs, inspect manifests/native libraries, and run
   emulator/device checks when the environment supports them. Clearly separate
   build/static/unit evidence from real YouTube, audio, device and performance
   acceptance. Record actual APK location/checksum and unresolved parity gates.
6. Add CI APK and desktop regression jobs, installation/development instructions,
   feature parity report and reproducible acceptance commands.

A blank tab, mocked extraction, generated demo feed or successful compilation is
not evidence of working real playback/downloads. Preserve all failures and open
gates in the Android validation/parity report rather than silently hiding them.
