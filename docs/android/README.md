# Android development preview (M9)

The Android app has a Rust engine and a Rust egui UI. GameActivity hosts the
render surface and keyboard. Small Android adapters use Media3 for hardware media
decoding, background playback, MediaSession notifications and durable downloads;
WorkManager prepares full playlists. There is no Python or mpv executable on
Android. The account sign-in web page is the only WebView screen.

This is a development preview, not a claim of full desktop parity or completed
Nothing Phone (3a) / Galaxy Tab S9 acceptance. Read [PARITY.md](PARITY.md) and
[VALIDATION.md](VALIDATION.md) before treating it as a daily driver.

## Install an APK

Open the repository's **Actions → Android → successful run → Artifacts** and
download `ytui-android-<commit>`. Extract the ZIP and install `app-debug.apk` using
Android's file manager, permitting installation from that source when prompted.
The APK includes ARM64 (both requested devices) and x86-64 (emulators), requires
Android 8/API 26 or newer, and targets API 35. Verify its SHA256 using the included
`SHA256-and-checks.txt`.

These APKs use development signing. Different CI runners can have different
keys: updates require the same signing key. Do not uninstall a working install
without exporting wanted data; uninstalling removes private account/library and
offline media. Stable, signed, in-place updates need a separately configured
release keystore and release process. No private signing key is checked in.

## Navigate

First installations default to 1080p with H.264 preference for mobile decoding;
existing choices are preserved. Choose Best explicitly for unrestricted formats.
Use the bottom tabs for Downloads, YouTube, Music and Radio. Search accepts words
or a media URL. **Feeds** selects provider shelves; categories and pagination use
server responses. The top **Menu** and **Settings** provide touch controls.
Tap a card's play, preview, queue, download or save action. Downloads → Add URL
accepts a video or dedicated playlist link, with audio-only and quality choices.
A watch link with `list=` keeps single-video intent. Tap the current player
to open transport/volume/speed, quality, audio/caption tracks, SponsorBlock, queue
and visualizer controls. Tablet layouts keep catalog and player side by side.
Android Back closes overlays before putting the app in the background.

Appearance follows Android light/dark or selects a built-in palette. Import a
Caelestia JSON, Omarchy TOML or exported ytui Palette JSON through the document
picker; Linux theme daemons are not accessed on a phone. Importing creates a
private copy. Vinyl/VU/Orbit are available only for Music/Radio. VU/Orbit use decoded
PCM levels, not microphone input; reduced motion and pause stop their animation.

Account access is explicit: in-app YouTube sign-in (which Google may reject in an
embedded browser) or a Netscape cookie file selected by the user. Another browser's
private database is inaccessible on normal Android. Disconnect clears the app's
session; local saved media/queue stay. Never send cookies to a bug report.

Android uses the system VPN normally. An explicit HTTP proxy can be set in
Settings; credential-bearing/SOCKS proxies are rejected on mobile rather than
routing only part of the app. With an explicit proxy, use cookie-file import
instead of WebView sign-in. HTTP radio stations are supported; HTTPS extraction
still verifies TLS. Update Android System WebView for the isolated challenge
solver, which has no browser, cookie or network APIs.

## Build and check

Install stable Rust, JDK 21, Android command-line tools, SDK platform/build-tools
35 and NDK 27.2.12479018. Accept SDK licenses yourself. Then:

```bash
rustup target add aarch64-linux-android x86_64-linux-android
export ANDROID_HOME=/path/to/android-sdk
./tools/android/build.sh
./tools/android/inspect-apk.sh
```

The build script installs nothing. `YTUI_ANDROID_ABIS=arm64-v8a` builds only your
phone ABI for development; the distributed universal APK needs both ABIs for the
inspection gate. `YTUI_ANDROID_RUST_ONLY=1` builds native libraries without Gradle.
Output: `apps/android/app/build/outputs/apk/debug/app-debug.apk`.
Native libraries go to ignored `target/android-jni`, with 16 KiB ELF segments.
The script includes application and Rust dependency license inventories. Android
libraries carry their original notices; review the complete dependency inventory
before a stable distribution.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
./tools/test-native-solver.sh # installed Node.js 22+, offline player vectors
./tools/android/test-fixtures.sh # installed FFmpeg, generated local media only
./apps/android/gradlew -p apps/android :app:testDebugUnitTest :app:lintDebug \
  :app:connectedDebugAndroidTest
```

Instrumentation needs an API 35 emulator with current Android System WebView or
an explicitly selected disposable test device. It changes that app's player,
volume, downloads and private library. Never point these tests at a personal
production install. It uses synthetic sessions and local generated media, not a
real account or public YouTube by default. For an explicit cookie-free public
check, dispatch the Android workflow with its `public_url` input, or run:

```bash
YTUI_ANDROID_PUBLIC_URL='https://www.youtube.com/watch?v=jNQXAC9IVRw' \
  ./tools/android/run-device-tests.sh
```

That additional test requires native extraction and three seconds of actual
decoding. A public-service failure fails the run; it is never replaced by a local
fixture pass. Public/provider and physical-device acceptance stay separate;
the desktop installer/tests remain unchanged in behavior.

## Repository structure

- `apps/desktop`: Rust terminal app, still `ytui` and `ytui-rs`.
- `apps/android`: packaging and Android platform adapters, no application UI.
- `crates/ytui-core`: shared models, configuration, themes and policy.
- `crates/ytui-providers`: shared networking, YouTube/Music/radio, sessions,
  bounded caches, images, lyrics and SponsorBlock.
- `crates/ytui-services`: desktop process, browser/keyring, mpv and OS adapters;
  re-exports shared providers for compatibility.
- `crates/ytui-mobile`: Rust Android engine, adaptive touch UI, GLES video,
  JNI boundary and PCM visualizer metering.
- `vendor/winit`: pinned 0.30.13 with a documented Android destruction/recreation
  adapter; other platforms retain upstream behavior.
- `vendor/eframe`: pinned 0.33.3 with documented Android suspended-window and EGL
  teardown handling so application cleanup runs without a caught Rust panic.
- `vendor/yt-dlp-rs`: pinned upstream source with a documented isolated-JS runtime
  callback; desktop Node/Deno behavior is retained. See its `YTUI_PATCH.md`.

## Explicit public YouTube acceptance

Use the Android workflow's **Run workflow → public_url** input for a public video
URL, for example `https://www.youtube.com/watch?v=jNQXAC9IVRw`. This invokes the
actual Rust extractor/Android JS bridge and requires three seconds of real Media3
playback. It imports no cookies. Default CI skips that separate live check;
rate limits, account/consent requirements and runner connectivity remain failures
in an opted-in run, not evidence of working public playback.
