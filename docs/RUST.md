# Rust edition

The Rust edition implements four provider tabs, a native download backend,
mpv playback, browser-cookie sign-in and desktop theme following. It is an alpha:
its automated tests are offline, and live signed-in behavior still needs checking
on CachyOS with Caelestia, Ghostty and Brave Origin. The repository application
and its screenshot/test tooling are Rust. [Remaining work](REMAINING_WORK.md)
was inventoried before removing the old Python application at the user's request.

## Build and run

Install current stable Rust, a C compiler, pkg-config and OpenSSL development
headers. On CachyOS/Arch:

```sh
sudo pacman -S --needed base-devel rustup openssl mpv ffmpeg
rustup default stable
git clone https://github.com/Vanir20342222/ytui.git
cd ytui
cargo run --release --locked -p ytui -- doctor
cargo run --release --locked -p ytui
```

Run `./install.sh` from the checkout to install both `~/.local/bin/ytui` and the
compatible `ytui-rs` launcher. `./install-rust.sh` calls the same Rust installer.
It builds before publishing and replaces an old launcher symlink itself without
writing through it into a virtualenv. It does not install system dependencies.
Every `ytui-rs` example below also works with `ytui`; with Cargo, use
`cargo run -p ytui --`.

```sh
ytui-rs --demo                         # fictional catalog, no browser access
ytui-rs --offline                      # local library; no account loading or online jobs
ytui-rs init                           # create ~/.config/ytui/rust.toml
ytui-rs doctor                         # dependencies and desktop integration
ytui-rs doctor --network               # public provider probes; no browser session import
ytui-rs --help
```

### Updating an older Rust alpha

`git pull` updates source files; it does not replace an installed `ytui-rs`.
Also, changes on an unmerged PR branch are not included when building `main`.
For the alpha.4 Rust-only application and general settings window, build this branch:

```sh
git fetch origin
git switch rewrite/rust-only-settings
git pull --ff-only
./install.sh
hash -r
~/.local/bin/ytui-rs --version          # must report 0.2.0-alpha.4
~/.local/bin/ytui-rs doctor --network
```

The explicit binary path uses the default install location; use your chosen
`YTUI_INSTALL_DIR` if different. `doctor` also prints the executable path to
identify a stale binary elsewhere in PATH. The installer replaces both launchers
with Rust and preserves installed data, settings and media.

`doctor` alone makes no provider requests. `doctor --network` runs anonymous
YouTube/Music searches and a fresh radio catalog/directory request. It reports
OK, EMPTY or FAILED separately, includes sanitized connection causes and does
not read browser stores, change VPN connections or validate private feeds.

Music bootstrap follows up to four redirects within its own origin. Redirects
to consent/sign-in pages require a browser session: open both YouTube and
YouTube Music in Brave Origin, complete any browser prompts, then import the
profile with `:auth`. ytui does not automatically accept consent or forward
credentials to another origin. A VPN exit can affect service availability;
use the public probes to distinguish connection failures from empty feeds.
Anonymous Home may have no recommendations; `/` opens public search and
`:auth` enables the personalized feed when the service supports that session.

When the cliamp service fails, a previously saved catalog is preferred. With
no saved cliamp catalog, ytui tries Radio Browser and labels the substituted
stations. This does not restore cliamp's track libraries during its outage.
Actual stream playback still depends on each station and mpv.

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
Native subtitle selection, sidecar downloads and FFmpeg embedding now use the
upstream Rust APIs. Language selectors support literal codes, `all` and exclusions;
regex selectors still require explicit fallback. Sidecars are retained alongside
media, including when embedded. Unavailable languages or unsupported embedding
produce visible errors and preserve existing files.

Native transfers now support byte pacing, SponsorBlock marking/removal, chapter
metadata and chapter splitting. These adapters remain subject to the extractor's
YouTube coverage limits. SponsorBlock uses only the explicitly selected categories
and its own HTTP client, without Google cookies. A 404 means no matching segments;
other failures remain visible. `sponsorblock_api` can select another compatible
server. FFprobe is required for chapter/SponsorBlock processing.

SponsorBlock removal re-encodes retained audio/video for accurate cuts: H.264
(CRF 18) for MP4/MKV or VP9 (CRF 18) for WebM, with the requested audio
format/quality. Resolution is preserved; re-encoding can change file size and
quality. Removal with requested or embedded subtitles requires explicit fallback
to avoid publishing captions with incorrect timing. Chapter splitting copies
streams, so boundaries can round to codec keyframes. Chapters come from existing
media metadata or timestamp lists in the description; for YouTube, the worker
also attempts the details provider. Missing/unsupported chapters produce an
error. Use explicit fallback for unsupported combinations.

Native outputs are staged privately, checked for filename collisions, then
published without overwriting existing files. Cancellation cleans staging. If a
later native postprocessor fails after a completed source was produced, the error
identifies the private `.ytui-job-*` recovery directory. Inspect/recover those
files before removing that directory manually. No job is marked complete merely
because the source transferred. Native template syntax is narrower than yt-dlp's;
the shipped template works on both backends.

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
| `b` | Focus Feeds on wide screens; open the feed picker on compact screens |
| `:` | Run a command/feed action |
| `Enter` | Open collection / play selected media |
| `v` | Muted ten-second mpv video preview |
| `u` | Resume selected media's local position, or show resume menu |
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

The Lazygit/Lazydocker-style workspace shows status, workspaces and feeds in a
left sidebar on screens at least 140 columns wide and 26 rows tall. `b` focuses
Feeds: arrows select, Enter opens, Esc returns to the catalog. `1`–`4` switch
workspaces. Smaller windows use the compact picker; `:feeds` always opens it.
Choices follow the current tab and offline mode offers local actions only.
Moving through feeds never starts playback or a download. Mouse clicks select
visible workspaces/feed entries/catalog items; scrolling follows the pane under
the pointer and dialogs block underlying clicks.

The workspace adapts to terminal size: wide layouts show selected-item details
and thumbnails, narrow layouts give the catalog the full width, and short windows
use a single-line player. Downloads show separate active/queued/paused/failed/
complete counts and a selected-item panel with progress, ETA, destination and
errors when space permits. Footer and selected-item shortcuts follow remapped
bindings. Dialogs hide thumbnail graphics while open; resume and watch dialogs
size to their contents and explain empty states.

The [WQHD gallery](UI_THEMES.md) has actual PTY captures for every bundled theme;
[the audit](FEATURE_AUDIT.md) lists verified behavior and remaining live checks.
Demo artwork is generated locally in Rust on a worker, without a thumbnail service.

Forms use `↑/↓` or `Tab` to change fields, `Space` for toggles, `Enter` to submit,
and `Esc` to cancel. Unicode editing and bracketed paste are supported. Typing in
an input never triggers the global letter shortcuts. Help reflects remappings and
scrolls with arrows/Page Up/Page Down. `:keys` edits bindings, including several
keys per action separated by spaces; duplicate assignments are rejected. Ctrl+C
remains the emergency quit key.

### General settings

Press `,` or enter `:settings` to open the settings window. Its eight sections are
General, Appearance, Downloads, Subtitles & media, Network & account, Schedule,
Tools and Radio. All 50 fields are reachable at compact sizes as well as WQHD.
PgUp/PgDn or Ctrl+Left/Right switches sections; F6 focuses the category list.
Up/Down selects a field, Left/Right cycles listed choices, Space toggles a switch,
and ordinary text fields support Unicode editing/paste. Save/Cancel and visible
fields/categories also accept mouse clicks. The compact view keeps the current
section and field visible while scrolling.

Enter or the Save button validates the whole draft before applying it. A parsing
or validation error leaves the draft open; Esc/Cancel discards edits. Save applies
new download defaults to new jobs and updates theme, image/network adapters and
player configuration. Existing jobs keep their captured options. Changing auth
storage copies the loaded session to the explicitly selected store; changing
account index validates that account before saving. `:auth` handles cookie import.

Defaults now include video/audio formats, resolution, audio quality, playlist
mode, duplicate handling and per-download/global bandwidth. Tools and Radio
include executable paths, pinned countries and directory servers. Reduced motion
limits redraws to ten frames per second; normal input renders at up to sixty.

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
:playlists ambient mixes
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

## Scheduling, clipboard and notifications

Settings expose daily download and bandwidth windows in local system time:

```toml
[schedule]
download_window = "22:30-06:00"   # blank field / omitted key means all day
bandwidth_window = "09:00-18:00"
window_bandwidth_kib = 512       # total cap during this window; 0 = unlimited
```

The normal total cap is `download.global_bandwidth_kib`. The window rate replaces
that cap while its window is active; a per-job cap can lower it further. The total
cap is divided conservatively over configured concurrency, in bytes, including
small limits. Native media/caption transfers are paced at the upstream read
boundaries (up to 64 KiB per read); cancellation remains prompt while waiting.
Metadata/cover requests are excluded from media-byte pacing. Explicit fallback
uses yt-dlp's own transfer limiter.

Closing the daily window stops/reaps active jobs and leaves them queued. Opening
it resumes eligible jobs; manual pauses remain paused. Changing the active total
cap restarts transfers so the new cap takes effect. Native transfers restart from
the beginning; fallback continuation follows yt-dlp's normal partial-file support.
No downloads run after ytui quits: keep the TUI running for scheduling.

`:schedule 30m URL`, `:schedule 2h URL` or `:schedule 90s URL` creates a persistent
delayed job. It also respects the daily window. Headless `download` refuses to
start outside that window unless `--now` is explicitly supplied; it snapshots the
rate at launch rather than running the TUI scheduler during a long transfer.
Recurring collection watches are described below.

Clipboard watching is disabled by default. Enable it in Settings or with
`:clipboard on`; disable with `:clipboard off`. It polls the regular clipboard
at two-second intervals using `wl-paste` on Wayland or `xclip`/`xsel` on X11.
Install `wl-clipboard` on CachyOS/Wayland. The first read establishes a baseline;
subsequent new YouTube URLs offer `:clipboard` to review download options. It
never automatically starts a download, reads the primary selection, or saves
arbitrary clipboard text. Unavailable readers back off for 30 seconds.

Desktop completion/failure notifications use the native freedesktop D-Bus API.
`notifications` controls them; `notification_sound` defaults to false. Sounds use
the notification server's sound hints and depend on that server's configuration.
Notification failure never changes download success. No notification tools or
Python modules are required. Actual desktop/sound acceptance is still pending.

## History, groups, profiles and import

`:downloads completed QUERY` searches completed titles/creators/URLs;
`:downloads failed`, `:downloads audio`, `:downloads video`, or `:stats` opens the
filtered history/statistics panel. `f` cycles filters, `Enter` jumps to the actual
queue row, and `g` expands its group. Statistics use recorded progress and paths;
they do not claim to measure current disk usage. `:history` in Downloads opens
completed downloads; in YouTube/Music it keeps its personal-feed meaning.

Open a collection and use `:download-page 1,3:7` or `:download-page all` to queue
individual entries from the **loaded page**, using the current download defaults.
This does not silently fetch or download unseen pages. `:groups` expands these
batches or migrated playlist groups while preserving entry order.

```text
:profiles
:profile save Music
:profile load Music
:profile delete Music
:import /path/to/queue.json
:export /path/to/queue.json
```

Profiles retain the complete native configuration and keymap. Loading goes through
normal validation/account-switch handling. Legacy profile JSON, unsupported values,
and original shortcuts are retained in migration backups and `legacy_settings` /
`legacy_keybindings`; preservation does not make unsupported fields active.

Queue import accepts Rust snapshots, legacy `items` JSON, URL arrays or one URL
per text line. It imports URL, mode, quality and title into **new queued jobs**;
other defaults come from your current config. It never imports executable hooks,
credentials or arbitrary output paths from an export. Invalid imports add nothing;
repeat imports skip duplicates. Headless `ytui-rs import FILE` saves the queue for
the next TUI session. `:clear`/removal delete records, never media files.

Legacy slash commands are also accepted when pasted/entered into the input
(e.g. `/history`, `/queue`, `/audio`, `/pauseall`, `/stopall`, `/profile`). Their
colon equivalents are available; `/history` always means download history.
`/log` / `:diagnostics` shows sanitized recent download/player errors. `/update`
shows the source-build update instructions; native extraction is compiled into
the application and no executable is downloaded/replaced by that command.

## Local resume, offline mode and collection watches

Playback positions stay in ytui's private state database. Press `u` on an item to
resume its saved position, or run `:resume` for the resume menu. `Enter` starts
normally from the beginning. In the menu, `Delete` forgets one position;
`:resume clear` forgets all. `playback_history = false` disables new recording.
Nothing plays automatically at startup and no watch history is sent to YouTube.
Previews/live stations are excluded. Signing in/out or changing accounts clears
online positions; positions for local files remain. Up to 500 entries are kept.

`:offline`, `:offline audio QUERY` or `:offline video QUERY` lists existing completed
media from the download database, excluding missing files and subtitle/image
sidecars. This is a catalog of recorded outputs rather than a recursive disk scan.
Thumbnails are omitted from this page so the catalog itself needs no network.

Start with `ytui-rs --offline` for local-only operation: account loading, initial
YouTube browsing, watch checks, remote thumbnail fetching, remote playback and
remote jobs are disabled. `/` searches local downloads. Saved online queue items
wait until an online session. Local playback/conversion remains available.
External application actions and configured user hooks retain their own behavior.

While viewing a collection/channel/playlist, run `:watch add 30m` to check it every
30 minutes (minimum one minute). The first successful **complete** catalog read
establishes a baseline and queues nothing. Later checks enqueue newly discovered
entries using the captured download defaults and normal duplicate checks.

`:watches` opens the watch manager: `Space` pauses/resumes, `Delete` removes the
watch, `Enter` opens its source. Commands `:watch pause UUID`, `:watch resume UUID`
and `:watch remove UUID` are also available. Watch data and baselines persist.
Account changes pause watches and reset baselines; resume explicitly afterward.
Offline mode suspends polling. Keep the TUI running for checks; no daemon is installed.

At most two checks run concurrently. A catalog must finish within 120 seconds,
32 pages, 2,000 entries and 16 MiB. Incomplete/failed polls update no baseline and
queue nothing; retries back off. Collections larger than these limits require
manual browsing/download selection. Live videos are excluded from automatic jobs.

## Optional VPN control

`:vpn list` lists existing NetworkManager VPN/WireGuard profiles with UUIDs.
`:vpn connect UUID` and `:vpn disconnect UUID` explicitly operate on one listed
profile. The same actions are available as `ytui-rs vpn list/connect/disconnect`.
NetworkManager handles authentication and permissions; ytui neither escalates
privileges nor reads/imports WireGuard private keys. It never probes or changes
VPN connections at startup and never disconnects on quit. `nmcli` must be installed;
its path can be set with `nmcli = "/path/to/nmcli"`.

This supports Proton configurations already imported into NetworkManager. It does
not implement the standalone Proton CLI login/server selector, raw wg-quick config
management, or Windows/macOS VPN helpers. Those legacy scope items remain visible
in PARITY.md. Automated tests invoke only a synthetic nmcli executable.

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
refreshing. `:pin-country DE`, `:unpin-country DE` and `:countries` manage saved
country shortcuts; browse one with `:country DE`. Location-based suggestions are
not implemented; no location lookup is made.

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
capability reporting is unreliable. Explicit protocol changes apply immediately;
switching back to `auto` uses half-blocks until the next startup capability query.
Moving app state with `--state-dir` does not relocate the desktop palette files.
Image decoding/encoding runs outside the
render loop, with request/size limits and a bounded disk cache.

## State, migration and rollback

Rust configuration: `$XDG_CONFIG_HOME/ytui/rust.toml`.
Rust state: `$XDG_STATE_HOME/ytui/rust/state.db`.
Thumbnail cache: `$XDG_CACHE_HOME/ytui/rust/images`.
Use `--config PATH` and `--state-dir PATH` for isolated installations/tests.
Alpha.2 advances the Rust state format to version 2 for positions/watches. On the
first version-1 upgrade it creates `state-v1-backup.db` using SQLite's backup API.
Older Rust alpha.1 refuses version 2; use the newer binary or an isolated copy of
that backup for Rust rollback. Python's separate state remains unchanged. The
state directory/database are private on Linux. After acquiring the instance lock,
ytui removes stale owned cookie exports/native manifests left by a forced exit;
it does not delete recovery/media directories.

```sh
ytui-rs migrate             # report only; no Python state is modified
ytui-rs migrate --apply     # back up and import supported fields
```

Migration imports queue/history order, paused state, group IDs, paths, supported
settings/shortcuts and profiles, including bandwidth schedules and desktop toggles.
Repeated imports do not duplicate jobs. An online
SQLite backup retains the source groups/schema/fields not represented by Rust's
current UI. Backups also preserve Python settings/shortcuts and existing Rust
configuration. Review [PARITY.md](PARITY.md) for deferred features and data fields.

The installed launchers now run Rust. Previous Rust builds need a compatible copy
of their database version for rollback. The old Python implementation is retained
in Git history and any existing legacy installation/data is left on disk. Do not
run both editions downloading to the same destination simultaneously. Rust's
separate state lock cannot coordinate another program's download directory.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# Requires installed FFmpeg/ffprobe; uses generated media and a loopback API only:
NO_PROXY=127.0.0.1,localhost cargo test -p ytui-services --locked real_ffmpeg_ -- --ignored
cargo build --release --locked
```

Tests use synthetic cookies, recorded/synthetic API shapes, fake processes, local
HTTP transfers and fake mpv IPC. No network account, browser database, desktop
keyring, audio device or live radio station is needed. Live-service and target
Linux checks are recorded separately in [VALIDATION.md](VALIDATION.md).
