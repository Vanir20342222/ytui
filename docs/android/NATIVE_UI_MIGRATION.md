# Native Android UI migration — 2026-10-28 (M9)

The user tested the preview on a Samsung Tab S9 and reported that text fields
cannot be typed into, theme selection has no visible effect, and a protected
feed shows desktop F2/CLI browser-import instructions. The user explicitly
allowed changing the stack. These are failed mobile UI acceptance cases, despite
earlier backend/fixture tests passing.

## Chosen change

Keep Rust providers, extraction, images, themes, account encryption, library,
queue/history, downloads and existing private data paths. Replace the production
egui/GameActivity UI with Jetpack Compose and a normal Android activity. This
supersedes the initial all-Rust UI choice with the user's current authorization.
The desktop terminal application stays Rust and retains its launchers/state.

Compose supplies Android input/IME, touch, accessibility, dialogs, adaptive
layouts and theme composition. Media3 renders video directly into an Android
surface with native captions; Rust still supplies extraction/policy and decoded
PCM visualizer measurements. Keep the monospaced terminal styling and all shared
palettes. No browser/web application UI is introduced.

The existing Rust UI can remain an optional prototype/test target during the
transition; it is not the production Android input/theme acceptance evidence.

## Required work

1. Document the decision and preserve the current backend/checkpoint evidence.
2. Add Compose hosting and a typed asynchronous Rust/JNI repository with request
   cancellation and separate feed/player/settings state. Keep all four tabs,
   shelf/category/continuation navigation, bounded image caches, queue/history,
   transport, player styles, downloads/export, lyrics and settings reachable.
3. Use Android text fields and immediate, persisted theme application. Keep
   system light/dark following and imported palettes. Preserve app data formats.
4. Put Sign in and Import cookies directly on account-required error screens.
   Translate provider errors for mobile instead of exposing F2/CLI instructions.
   Anonymous search remains accessible. Android cannot read another browser's
   private cookie store; offer explicit app sign-in and document/header import.
5. Add explicit sign-in completion/cancel controls, without a JavaScript host
   bridge or importing on activity destruction/rotation. Do not claim Google
   always accepts embedded sign-in.
6. Replace legacy UI lifecycle/GLES assertions with real activity/input/theme,
   Compose navigation and Android video-surface checks. Retain backend JNI,
   solver, decoding/PCM, queue/history, export, encryption and radio regressions.
7. Build both ABIs, lint and run accelerated emulator tests. Check typed search,
   keyboard visibility, all theme changes, account actions, phone/tablet layouts
   and actual generated video rendering. Qualify the result on the user's Tab S9;
   live YouTube/network and stable owner signing remain separate gates.

Do not recommend the replacement APK or mark this migration complete until its
new runtime tests pass. Do not count the prior preview's fixture checks as proof
that its physical-device UI worked.
