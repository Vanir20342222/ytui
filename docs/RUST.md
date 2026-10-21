# Rust edition

The Rust edition has four working provider tabs, a native download backend,
mpv playback, browser-cookie sign-in and desktop theme following. It is an alpha:
its automated tests are offline, and live signed-in behavior still needs checking
on CachyOS with Caelestia, Ghostty and Brave Origin. The original Python release
remains runnable while those acceptance gates are completed.

## Build and run

Install current stable Rust, a C compiler, pkg-config and OpenSSL development
headers. On CachyOS/Arch:

```sh
sudo pacman -S --needed base-devel rustup openssl mpv ffmpeg
rustup default stable
git clone --branch rewrite/rust https://github.com/Vanir20342222/ytui.git
cd ytui
cargo run --release --locked -p ytui -- doctor
cargo run --release --locked -p ytui
```

Or run `./install-rust.sh` from the checkout. It installs `~/.local/bin/ytui-rs`
alongside Python's `ytui`. It does not install system dependencies or modify the
Python installer. Every example below uses `ytui-rs`; with Cargo, replace it with
`cargo run -p ytui --`.

```sh
ytui-rs --demo                         # fictional catalog, no browser access
ytui-rs init                           # create ~/.config/ytui/rust.toml
ytui-rs doctor                         # dependencies and desktop integration
ytui-rs --help
```

Linux is the supported implementation target. macOS/Windows packaging and
platform playback adapters are not implemented; the player uses Unix sockets.

## Native Rust and the explicit fallback

`download_backend = "native"` is the default for downloads and YouTube playback.
The application links [aagedal/yt-dlp-rs](https://github.com/aagedal/yt-dlp-rs), pinned
to revision `10223c019ef6ec5552627b90251d8e967404e440`. It executes in a supervised
copy of the Rust binary so blocking extraction and child FFmpeg jobs can be
terminated. It does not invoke Python. Browser imports use the Rust rookie crate.

Upstream's native YouTube extractor remains an early prototype. Many current
videos, token-restricted streams, live YouTube, YouTube playlists and persistent
cross-process transfer resume are unsupported. Native pause stops the operation;
resume starts that transfer again. The yt-dlp fallback can continue its own
partial downloads when the source supports it. A partial created by one backend
is not transferred to the other backend.

YouTube signature/n challenges may require an explicitly installed Deno runtime.
The pinned upstream expects Deno 2.9.7 for its bundled solver; configure its actual
executable path with `native_js_runtime = "/usr/bin/deno"`. A runtime is only
launched when needed. Deno and FFmpeg are external tools, not Python dependencies.

**There is no automatic fallback.** To use the Python yt-dlp executable deliberately:

- Press `Y` on a failed/paused download to retry that job with yt-dlp.
- Press `Y` on a video/song to play it through mpv's yt-dlp hook.
- Set a download form's backend to `yt-dlp` for that job.
- Run `ytui-rs download URL --backend yt-dlp`.
- Set `download_backend = "yt-dlp"` to select it globally.

Install yt-dlp separately for these actions. Native-only use does not require it.
Settings choose the backend for new playback, previews and download jobs. Existing
jobs retain any explicit backend override.

Native download support includes direct HTTP media, the upstream HTML5/YouTube
subset, separate-stream merging, audio conversion, remuxing and metadata tagging.
Native SponsorBlock, chapter splitting, bandwidth caps and subtitle controls are
currently rejected with an actionable fallback message; they are not silently
ignored. The fallback implements these download controls. Native template syntax
is narrower than yt-dlp's; the shipped template works on both backends.

## Your browser and account

Sign in to YouTube in the selected browser first. Discovery lists directories
without opening browser cookie databases:

```sh
ytui-rs auth profiles
ytui-rs auth import --browser brave-origin --profile "$HOME/.config/BraveSoftware/Brave-Origin/Default"
ytui-rs auth status
```

Brave's [Linux path implementation](https://github.com/brave/brave-core/blob/master/chromium_src/chrome/common/chrome_paths_linux.cc)
uses `BraveSoftware/Brave-Origin` for Origin, with optional channel suffixes.
Automatic Origin selection works only when discovery finds exactly one profile;
otherwise select one explicitly. `brave://version` shows the actual Profile Path.
Brave Origin decryption on the user's keyring still needs local verification.

Other supported import routes include Firefox, Librewolf, Chrome, Chromium, Edge,
Vivaldi, and custom profile paths for Zen. Flatpak Brave/Firefox roots are included
in directory discovery. Not every browser packaging/keyring combination is tested.

In the TUI, `:auth` opens the same setup form. Choose a browser/profile or a
Netscape cookie file. Candidate credentials are scoped to relevant Google/YouTube
domains, checked for a valid session, and validated against the selected YouTube
account before replacing a stored session. Existing credentials remain intact
when import/validation fails. A locked desktop keyring must be unlocked by the user.

The default store is the desktop keyring. For an explicitly chosen local-file
alternative:

```sh
ytui-rs auth import --cookies /path/to/cookies.txt --file-store
```

This saves credentials in a private directory with atomic `0600` files and records
`auth_store = "file"` in the config. Temporary child-tool cookie files are private
and removed after normal completion/cancellation. Forced machine/process failure
can leave private temporary files; they are never committed or logged.

`:disconnect` or `ytui-rs auth disconnect` removes the selected ytui session store.
It does not sign the browser out. Disconnect also clears the TUI's account-scoped
pages and playback queue. CLI disconnect takes effect in another running TUI when
it is restarted. Do not share cookie files, raw API fixtures, or signed stream URLs.

`account_index` selects the browser's YouTube account/channel index. Changing it
in the TUI validates the candidate account and invalidates old account pages and
requests. Separate simultaneously signed-in ytui profiles are not yet supported.

## Navigation

| Key | Action |
| --- | --- |
| `1`–`4`, `Tab`, `Shift+Tab` | Switch tabs |
| Arrows or `j` / `k` | Select an item |
| `/` | Search, or paste a URL to open download options |
| `:` | Run a command/feed action |
| `Enter` | Open collection / play selected media |
| `v` | Muted ten-second mpv video preview |
| `Esc` | Close preview/modal or go back |
| `d` | Download options |
| `Space` | Pause playback; pause/resume selected job in Downloads |
| `Delete`, `r`, `Ctrl+↑` / `Ctrl+↓` | Remove, retry or reorder downloads |
| `Y` | Explicit yt-dlp retry/playback fallback |
| `e`, `p` | Add to playback queue / show queue |
| `[` / `]`, `←` / `→` | Previous/next track, seek ±5 seconds |
| `+` / `-`, `m` | Volume, mute |
| `s`, `R`, `x` | Shuffle, cycle repeat, stop |
| `f` | Favorite selected radio station |
| `n`, `Ctrl+r` | Load more / refresh |
| `t`, `,`, `?` | Themes, settings, contextual help |
| `C` | Suspend ytui and run installed cliamp |
| `q`, `Ctrl+c` | Clean shutdown |

Forms use `↑/↓` or `Tab` to change fields, `Space` for toggles, `Enter` to submit,
and `Esc` to cancel. Unicode editing and bracketed paste are supported. Typing in
an input never triggers the global letter shortcuts. Help reflects remappings.

```toml
[keybindings]
"V" = "preview"
"ctrl+d" = "download"
"f1" = "help"
```

These move the action's default binding to the supplied key. Valid action names
are defined in `ytui-core/src/shortcuts.rs`. Unknown actions/key names are errors.

## YouTube and Music

The browse and Music tabs have separate WEB/WEB_REMIX request contexts. Signed-in
feeds use the selected account's browser session, not a generic video search
standing in for a personal library. Reads use an isolated internal API adapter;
YouTube service changes can require parser updates. Access is read-only: no hidden
likes, subscription changes, playlist mutations or watch-history uploads.

Useful commands:

```text
:home
:search query
:subscriptions
:later
:liked
:library
:history
:details
:related
:playlist PLAYLIST_ID
:channel CHANNEL_ID
:songs ambient piano
:albums live sessions
:artists artist name
:albums
:artists
```

The final two open the signed-in Music album/artist libraries. `:library` in Music
opens playlists; `:liked` opens liked songs. Collection pages are navigable with
`Enter`; `n` loads another page and `Esc` restores the previous page and selection.
`:details` loads the selected video's description and metadata. Thumbnails render
for the selected item in wide layouts; narrow terminals retain a usable text list.
There is no in-terminal moving video; previews and full video use mpv windows.

## Downloads and local conversion

```sh
ytui-rs download 'https://example.com/video.mp4'
ytui-rs download URL --audio --directory "$HOME/Music/ytui"
ytui-rs download PLAYLIST_URL --playlist --items '1,3:7' --backend yt-dlp
```

The download form chooses audio/video, height, containers, destination, playlist
selection, subtitles, metadata, cover art, rate limit and backend. It snapshots
these options per job. A video URL is single-video by default; a whole playlist is
an explicit choice. Download progress can have unknown size or ETA. Completion
requires a successful child exit, successful postprocessing and existing final
output files. Download history remains visible after restart; `:clear` clears
completed rows without deleting media. `:export PATH` exports queue/history JSON.

`:convert "/path/to/local file.mkv"` opens conversion options for a local file.
FFmpeg writes a separate output and never changes the input. Existing conversion
destinations are not overwritten. Partial conversion files can be removed manually.

Transient network failures have bounded, cancellable backoff; permanent errors are
not retried automatically. Pause/removal invalidates older worker results before
they can touch the queue or persistence. A single-writer lock protects the state
DB, and a dedicated DB thread owns its connection. Normal shutdown reaps owned
process groups before closing the database.

## Radio and cliamp

The radio tab fetches live cliamp channel metadata from its service. `Enter` opens
a channel's track library when provided, or plays a live-only station. `:live`
plays a selected channel's continuous stream. Track libraries support normal queue
controls; live streams reconnect with bounded delays. mpv supplies stream metadata.

```text
:cliamp
:radio station name
:country DE
:tag jazz
:favorites
:custom
:live
```

Radio Browser uses server discovery/failover and country/tag/name filtering.
Favorite stations persist locally. Custom HTTP(S) stations can be configured in
`rust.toml` or imported without changing cliamp's files:

```sh
ytui-rs import-radio "$HOME/.config/cliamp/radios.toml"
ytui-rs import-radio "$HOME/.config/cliamp/radio_favorites.toml"
```

`C` runs cliamp as a separate installed application while ytui's terminal is
suspended. This is not a full reimplementation of cliamp's unrelated providers.
Catalogs are cached to disk and used with a visible notice when the service is
unavailable. Existing loaded pages and saved favorites remain usable while
refreshing. Station-country pinning and location suggestions remain in the backlog.

## Themes and terminal images

`t` opens curated dark/light themes. `:theme NAME` applies a named theme, `system`
follows the desktop, and `terminal` uses terminal ANSI colors. A custom palette
file uses Omarchy-style `background`, `foreground`, optional accent/status colors.

System providers are `auto`, `caelestia`, `omarchy`, `portal` and `gnome`:

- Caelestia: `$XDG_STATE_HOME/caelestia/scheme.json`, mapping Material color roles.
- Omarchy: `$XDG_STATE_HOME/omarchy/current/theme/colors.toml`, with the older
  `$XDG_CONFIG_HOME/omarchy/current/theme/colors.toml` fallback and `light.mode`.
- GNOME/KDE/other compatible desktops: Settings portal light/dark/accent hints;
  GNOME GSettings is an available fallback. These hints are not a full GTK palette.

Palette files are polled every two seconds, covering symlink and atomic directory
replacement. Portal signals also trigger updates. Invalid intermediate files keep
the last valid palette. Theme scripts are never executed and desktop settings are
not modified. Manual theme selection overrides automatic following.

`image_protocol` supports `auto`, `kitty`, `sixel`, `iterm2`, `halfblocks`, `none`.
Auto detection queries the terminal; half-block rendering is the fallback. Ghostty
uses the Kitty graphics protocol. For tmux/SSH, set an explicit protocol if
capability reporting is unreliable. Image decoding/encoding runs outside the
render loop, with request/size limits and a bounded disk cache.

## State, migration and rollback

Rust configuration: `$XDG_CONFIG_HOME/ytui/rust.toml`.
Rust state: `$XDG_STATE_HOME/ytui/rust/state.db`.
Thumbnail cache: `$XDG_CACHE_HOME/ytui/rust/images`.
Use `--config PATH` and `--state-dir PATH` for isolated installations/tests.

```sh
ytui-rs migrate             # report only; no Python state is modified
ytui-rs migrate --apply     # back up and import supported fields
```

Migration imports queue/history order, paused state, group IDs, paths, supported
settings/shortcuts and profiles. Repeated imports do not duplicate jobs. An online
SQLite backup retains the source groups/schema/fields not represented by Rust's
current UI. Backups also preserve Python settings/shortcuts and existing Rust
configuration. Review [PARITY.md](PARITY.md) for deferred features and data fields.

Rollback is simply using the Python launcher and its original state again. Do not
run both editions downloading to the same destination simultaneously. Rust's
separate state lock cannot coordinate another program's download directory.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
```

Tests use synthetic cookies, recorded/synthetic API shapes, fake processes, local
HTTP transfers and fake mpv IPC. No network account, browser database, desktop
keyring, audio device or live radio station is needed. Live-service and target
Linux checks are recorded separately in [VALIDATION.md](VALIDATION.md).
