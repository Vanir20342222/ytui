# Android development preview (M9)

The Android app has a shared Rust engine and a native Jetpack Compose UI with
artwork-first media grids, sans-serif typography and rounded native controls. Android supplies text fields, keyboard, touch and
adaptive layouts. Media3 handles hardware video/captions, background playback,
MediaSession notifications and durable downloads;
WorkManager prepares full playlists. There is no Python or mpv executable on
Android. The account sign-in web page is the only WebView screen.

This is a development preview, not a claim of full desktop parity or completed
Nothing Phone (3a) / Galaxy Tab S9 acceptance. Read [PARITY.md](PARITY.md) and
[VALIDATION.md](VALIDATION.md) before treating it as a daily driver. The
[device acceptance checklist](DEVICE_ACCEPTANCE.md) covers the native UI re-test.

The [tested native UI APK and checksum](VALIDATION.md#tested-development-apk)
cover source `4b2e988`: 17 emulator checks pass, including actual keyboard input,
all-theme contrast, account navigation and visible generated video. Real device/
YouTube acceptance is still required; earlier failed APKs are historical only.

## Install an APK

The [tested native UI preview](https://github.com/Vanir20342222/ytui/releases/tag/android-native-preview-2026-10-28)
provides a direct `.apk` download without GitHub sign-in or ZIP extraction.
Download the APK under **Assets**, open it from Android's Files app, and permit
installation from that source when prompted. Its checksum and inspection report
are attached. This publishes the exact tested artifact without rebuilding or
changing its signing identity; it remains a development preview.

For other CI builds, sign in to GitHub in your browser first.
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
The manual [signed-build workflow and identity setup](SIGNING.md) provide that
build path without committing secrets; an owner must configure the identity.

## Navigate

First installations default to 1080p with H.264/AAC preference for mobile decoding
and portable file export; audio-only playback still selects the best audio.
Existing quality choices are preserved. Choose Best explicitly for unrestricted formats.
Use the bottom navigation for YouTube, Music, Radio and Downloads. Search accepts words
or a media URL. **Feeds** selects provider shelves; categories and pagination use
server responses. The top **⋮** menu, **Settings** and **Sign in / Account** provide touch controls.
Tap artwork to play/open it. The card's **⋮** menu contains preview, queue, save
and download actions. Downloads → Add URL
accepts a video or dedicated playlist link, with audio-only and quality choices.
A watch link with `list=` keeps single-video intent. Tap the current player
to open transport/volume/speed, quality, audio/caption tracks, SponsorBlock, queue
and visualizer controls. Tablet layouts use a navigation rail and keep the catalog and active player side by side.
Android Back closes overlays, then returns to the previous feed with its scroll
position, before putting the app in the background. Each tab retains up to twelve
navigation steps and 2 MiB of cached previous pages. Disconnecting clears account navigation history. The queue's
arrow buttons reorder items; edits are saved before their result is shown.

Music albums/playlists and cliamp track libraries continue automatically after
the selected song. The full finite collection prepares in the background; manual
queue entries play first. Menu → Recently played opens local playback history.
Previous restarts a track after three seconds, otherwise returns to the prior
played item; Next follows the manual queue, forward history, then collection.
Notification/media-session controls use those same actions. History is local and
can be disabled or cleared with resume positions. A stopped/killed process does
not retain derived collection context; select the collection again to rebuild it.

Menu → **Lyrics** opens a song-specific view. Review title and artist
before choosing Find on LRCLIB; no automatic lookup or cookies are sent. Choose
a returned version or import a UTF-8 LRC/plain text document (up to 512 KiB).
Enable **Follow playing line** for automatic scrolling; timed lines support tap-to-seek. Positive Delay shows lines
later; edits and adjustments save locally for that media URL. Timing pauses when
another song is playing. The local lyric library holds up to 100 songs / 8 MiB;
Remove saved lyrics frees an entry without changing saved media or the queue.

For a completed offline download, tap **Export file…** and choose a
destination in Android's document picker. Original MP4/M4A/WebM containers are
copied without re-encoding; compatible separate video/audio tracks are combined
into one MP4 or WebM file. Export runs in a foreground worker, reads only completed
offline cache data and does not contact the original server. Export jobs appear
in Downloads; Remove cancels/dismisses a job, preserving a completed exported file.
Segmented streams, live recordings and incompatible codec combinations cannot
currently be exported. MP3/FLAC conversion is not offered as an implemented option.

Appearance follows Android light/dark or selects a built-in palette. Import a
Caelestia JSON, Omarchy TOML or exported ytui Palette JSON through the document
picker; Linux theme daemons are not accessed on a phone. Importing creates a
private copy. Vinyl/VU/Orbit are available only for Music/Radio. VU/Orbit use decoded
PCM levels, not microphone input; reduced motion and pause stop their animation.

Account access is explicit: in-app YouTube sign-in (which Google may reject in an
embedded browser), a Netscape cookie file selected by the user, or an explicitly
pasted Cookie header. Use **Sign in / Account** in the top bar, or the account
action on a protected-feed error. After signing in, choose **Use this session**;
**Cancel**, rotation or closing the page never imports implicitly. Another browser's
private database is inaccessible on normal Android. Disconnect clears the app's
session; local saved media/queue stay. Never send cookies to a bug report.

Android encrypts persisted sessions using AES-GCM and a non-exportable key in
Android Keystore. Earlier private plaintext imports migrate on load; if cleanup
fails the app reports it. New document imports parse in memory without a plaintext
temporary file. Account → Reload stored account retries the saved envelope. A
missing/invalid key or damaged envelope clears active authentication and keeps the
offline library usable; reconnect/import or disconnect to recover. Hardware key
backing depends on the device. The app never falls back to new plaintext storage.
Native extraction builds its cookie jar in memory too. Recognized stale private
cookie staging files from earlier builds are removed at startup; unrelated files
are preserved and a cleanup failure is reported.

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
