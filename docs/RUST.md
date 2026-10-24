# Rust edition

Alpha.12 provides Downloads, YouTube, Music and Cliamp / Radio in a Rust terminal
application. mpv plays video in its own window and audio through the shared player.
CachyOS + Caelestia + Ghostty + Brave Origin is the primary desktop target.

The alpha.12 [workflow guide](AUDIT_FEATURES.md) covers the profile chooser,
collection jobs, available formats/tracks, cache controls, MPRIS and support export.
Use [candidate/release instructions](RELEASE_CANDIDATE.md) for checksum/rollback
and exact-SHA Arch builds. The candidate is integrated on main; target acceptance remains open.

## Build and run

On CachyOS/Arch:

```sh
sudo pacman -S --needed base-devel rustup openssl mpv ffmpeg nodejs
rustup default stable
git clone https://github.com/Vanir20342222/ytui.git
cd ytui
./install.sh
hash -r
~/.local/bin/ytui --version
~/.local/bin/ytui doctor
```

The installer builds Rust and installs `ytui` and `ytui-rs` in `~/.local/bin`, or
`YTUI_INSTALL_DIR`. It builds before replacing launchers, replaces an old symlink
itself, and preserves settings, queues, history, browser profiles and media.
`install-rust.sh` is a compatibility entry point. No system package is installed
by these scripts. Native extraction can run without external yt-dlp; auto fallback
requires that tool and its supported JavaScript runtime.

### Updating an older Rust alpha

Update from main. Pulling source does not replace your installed executable:

```sh
cd ~/ytui
git fetch origin
git switch main
git pull --ff-only
./install.sh
hash -r
~/.local/bin/ytui --version             # 0.2.0-alpha.12
type -a ytui ytui-rs
```

Playback and downloads have independent backends. An older configuration with
only `download_backend` now uses native playback; an explicit `playback_backend`
is preserved. General settings → Playback selects playback extraction; Tools
selects download extraction and the native JavaScript runtime. Node.js 22+ is
detected automatically; clear an old unsupported Deno path to use detection.
Install external `yt-dlp` separately for auto/external modes or explicit `Y` retry.
Existing jobs retain their own backend overrides. `doctor` prints executable and
configuration paths to identify another installation earlier in PATH.
See [the example configuration](../examples/config.toml) for the current defaults.

## Navigation and settings

Alpha.8 shows contextual recovery instructions for playback/download failures.
`doctor --network` returns nonzero if any public probe fails or has no supported
items. Preview controls preserve the main player's saved volume/mute/speed and
pause intent. [Stability validation](STABILITY_PASS.md) lists checks and limits.

| Key | Action |
| --- | --- |
| `1`–`4`, Tab / Shift+Tab | Workspaces |
| Arrows or `j` / `k` | Select an item |
| `b` | Feed/source list; Enter opens the selected feed |
| `/` | Centered search / URL form |
| `F2` | Searchable Actions panel; type words, arrows choose, Enter opens |
| `,` | General settings |
| `F1` | Show/hide bottom shortcut hints; stored in configuration |
| `?` | Keyboard and navigation help |
| `Enter` | Play a video/song/live station, or open a collection |
| `v` | Explicit muted ten-second low-resolution mpv preview |
| `d` | Categorized download options |
| `F3` | Categories supplied by the current YouTube/Music response |
| `Esc` | Close/back; restore previous page and selection |
| `n` | Explicitly load another page; near-end scrolling also prefetches |
| `u` | Resume a locally saved playback position |
| `e`, `p` | Add media to queue / open queue |
| `l` | Open selected station's separate track library |
| `Y` | Explicitly retry/play selected media through yt-dlp |
| `q`, Ctrl+C | Quit |

The bottom command entry has been removed. `:` is a compatibility key for the
Actions panel. Accounts, feeds, Music category search, downloads/history, watches,
profiles, VPN, diagnostics and updates have labeled actions; actions needing a
path, query, name or ID open a form. Search input never runs arbitrary commands.
Internal action routing retains migration compatibility; text in the search form
is always a query or URL. Headless CLI subcommands remain available.

General settings contains **nine sections / 54 fields**: General, Appearance,
Downloads, Subtitles & media, Network & account, Schedule, Tools, Radio, Playback.
Download options has three sections: Quality & format, Files & playlist, Captions
& metadata. Wide forms space fields vertically; compact forms keep selected fields
visible. PgUp/PgDn or Ctrl+Left/Right changes section; F6 focuses categories;
Up/Down or Tab changes field, Left/Right cycles choices, Space toggles switches,
Ctrl+U clears text, Enter validates/applies, and Esc cancels. Fields, categories,
Save/Cancel also accept mouse clicks. Validation errors preserve the draft.

Defaults apply to new jobs. Auth storage changes copy the currently loaded session
to the chosen store; account changes validate before saving. Reduced motion limits
redraws to ten frames per second. Normal input is capped at sixty; idle player
status does not force constant redraws. State writes wait independently of input.
Keys can be remapped through F2 → Keyboard bindings or `[keybindings]` in TOML.
Ctrl+C is reserved for emergency quit.

## Automatic feed recovery

YouTube and Music recover transient feed failures without pressing Refresh. The
active tab retries after 2, 5, 15, 30, then 60 seconds (remaining at 60); server
`Retry-After` delays take priority. Hidden tabs wait until revisited. Offline/demo
mode does not start retries. Loaded videos remain usable during recovery, and a
same-feed refresh preserves the selected video where it is still present.

Fresh Home data embedded in the bootstrap HTML is used once, avoiding an extra
API round trip. Empty bare Home shells renew the client context once. WEB and
WEB_REMIX contexts remain separate, expire after 30 minutes, and renew once on
400/412 rejection. Each fetch has a 35-second overall deadline. Session expiry,
consent and HTTP 401/403 require the displayed account/network action rather than
repeated requests. Anonymous Home may legitimately have no recommendations.

Near-end prefetch also considers visible cards. Overlapping or empty continuation
pages with new tokens can advance; repeated/cyclic tokens and three consecutive
pages without new items stop the loop. Refresh/Next do not restart pending loads
or bypass their cooldown. Feed recovery never writes watch history to YouTube.

## Native Rust playback and separate download fallback

Playback defaults to `playback_backend = "native"` following the later native-only
watching request. Native [yt-dlp-rs](https://github.com/aagedal/yt-dlp-rs), pinned at
`f67d96bdf98c5b821d0c9bf6175d328a0fa8ceab`, runs in a supervised copy of the Rust
application. It accepts native HLS manifests and forwards extractor headers to
mpv. Interrupted CDN streams can renew their URL once and resume locally; bot,
account and proxy failures do not enter that retry loop. Native mode never starts
external yt-dlp. See [current playback checks and limits](SINGLE_VIDEO_SWEEP.md)
and [hosted-runner CI diagnosis](CI_PLAYBACK_DIAGNOSIS.md).

Downloads retain `download_backend = "auto"`. Playback can opt into auto separately.
Both settings offer:

- `auto`: Rust first, yt-dlp for supported extractor limitations.
- `native`: strict Rust extraction, with actionable failures and no external fallback.
- `yt-dlp`: explicitly use the external extractor immediately.

`Y` is still available for one explicit external retry. Per-job backend choices and
`ytui download URL --backend auto|native|yt-dlp` override the global default.
Cancellation and local publication/disk failures do not trigger an external retry.

Alpha.7 resolves external playback directly with yt-dlp, then hands validated video/
audio URLs and the extractor's HTTP headers to mpv with its extraction hook disabled.
Cookies, proxy, quality, audio-only and preview selections are passed explicitly.
This avoids dependence on mpv's bundled yt-dlp hook and its option translation.
mpv output is captured, including stdout errors, so failures show whether extraction
or opening the stream failed. **F2 → Diagnostics** shows the current player error;
signed URLs are redacted. For external extraction, Deno must be discoverable by
installed yt-dlp (normally through PATH); the pinned native runtime setting is separate.

Auto fallback is not a solution to every VPN, account, consent or bot restriction.

The native solver detects installed **Node.js 22+**, then **Deno 2.9.7**, for its
supported signature/n challenges. Settings → Tools → Native JavaScript runtime
overrides detection; a blank field restores it. Explicit unsupported versions
produce an actionable error. A newer distribution Deno may only be compatible
with external yt-dlp. JavaScript runs locally with restricted permissions; no
executable or browser session is fetched automatically. Upstream still has
account/live/DRM limitations, and ytui native playlist downloads stay gated.
Pausing a native transfer preserves intent but restarting it may repeat transfer;
yt-dlp can continue its own partials. Partials are not moved between backends.

## Playlist and music downloads

Paste a dedicated YouTube/Music `…/playlist?list=…` link: the download form enables
Whole playlist, and pasting in Music selects audio. In the CLI:

```sh
ytui download 'https://music.youtube.com/playlist?list=PLAYLIST_ID' --audio --backend auto
```

A watch link containing both v= and list= remains a single video unless you choose
Whole playlist or pass `--playlist`. Playlist indices restrict the collection, for
example `--items '2,3'`. Auto uses external yt-dlp for unsupported native YouTube
playlists; strict native reports that limit. Current per-entry metadata cannot
replace the saved collection source. Partial completed outputs remain recorded
when a later entry fails, and the job stays failed until successfully retried.

Re-add the original playlist URL after upgrading if an older job already points
to only its first song. Existing files are preserved. A standalone music-download
pipeline is covered by real FFmpeg conversion; live YouTube extraction and the
reported desktop/VPN still need acceptance.

## Browsing and thumbnails

YouTube and Music use separate WEB / WEB_REMIX contexts. Browser-session import
is explicit: **F2 → Account → Connect browser or cookie file**. Select Brave Origin
and the actual Profile Path from `brave://version`, or a Netscape cookie file.
Cookies stay on the user's machine, scoped to approved YouTube/Google origins.
The default store is the desktop keyring; `auth_store = "file"` uses private files.
F2 → Disconnect removes ytui's saved session and account caches, leaving the browser.

Home, subscriptions, Watch Later, liked media, library and history depend on what
the selected account/service returns. Anonymous Home can be empty; `/` offers
public search. Account access is read-only: no likes, playlist edits, subscriptions
or automatic watch-history uploads. Consent or cross-origin sign-in redirects
require completing that flow in the browser and importing the session again.

Home/search cards retain server section labels. Category chips and F3 follow the
actual server-provided endpoint/parameters; available categories differ by account,
feed and region. Unsupported response shapes can still require provider updates.
When selection approaches the last twelve loaded items, a continuation loads in
the background. Appending keeps selection and existing content, deduplicates IDs,
and stops repeated/empty tokens. Failures wait for a deliberate refresh/retry.

The catalog gets 74% of the browse width on wide terminals, with inline image
cards and a smaller details pane. Music artwork uses a square region. Small
terminals keep a usable list. Thumbnails choose the smallest advertised size
covering the displayed pixels; supported dynamic image origins receive size
parameters. Nearby images prefetch with bounded concurrency. Decoded images are
cached in memory (96 entries / 32 MiB), resized PNGs on disk (128 MiB). Disk cache
survives restart and provider outages. Cold images still need their first fetch;
a quiet placeholder replaces the loading text. Images never receive account cookies.

## Player, radio and SponsorBlock

mpv's mouse controller remains available. Press **g in the mpv window** for the
bundled menu: quality, audio/subtitle tracks, subtitle visibility, speed, chapters,
SponsorBlock and fullscreen. Quality changes re-resolve the source and resume near
the previous position while keeping a paused video paused. The window menu uses modern mpv's `user-data` property;
use current mpv (0.38 or later). Hardware decoding and native window video remain
mpv's responsibility. An in-terminal renderer such as mpv `tct` has lower image
quality and a larger terminal/CPU cost; it is not enabled by this change.

In the terminal, `o` opens player actions; the player strip has clickable controls
and a clickable seek timeline. `Space` pauses (in Downloads it controls the selected
job), Left/Right seeks five seconds, `+`/`-` changes volume, `m` mutes, `[`/`]`
changes track, `<`/`>` changes speed, `a` changes audio, `c` changes subtitles, `V`
toggles subtitles, Ctrl+Left/Right changes chapter and `z` toggles fullscreen.
`s` shuffles, `R` cycles repeat, `x` stops. Volume, mute and speed persist privately
in `player.json` and carry across tracks/stations/restarts. Status polling uses
one bounded IPC batch; commands are prioritized over the polling tick.

**Enter on a radio station plays its continuous live stream.** `l` opens its track
library if exposed. Playing a song queues the subsequent loaded songs, advancing
on completion. Live stations reconnect with bounded delays. Favorites, country/tag
search and Radio Browser failover are available through the source list/Actions.
Custom HTTP(S) stations can be added through Actions or imported read-only:

```sh
ytui import-radio "$HOME/.config/cliamp/radios.toml"
ytui import-radio "$HOME/.config/cliamp/radio_favorites.toml"
```

The optional installed cliamp program launches through a separate action while
ytui suspends/restores its terminal. Its unrelated providers are outside this port.

Direct extraction plays media without the YouTube website's ad player. This is the
ad-free playback path; it does not promise removal of server-side ads embedded in
a stream. **Playback SponsorBlock** defaults to `skip` with category `sponsor`.
Settings → Playback supports `skip`, `mark`, `off` and a category list. API requests
contain only the public video ID and selected categories, never cookies/signed URLs.
Segments appear in a contrasting timeline color. `B` toggles skipping for the
current media; `U` replays the last skipped range without immediately skipping it
again. Overlapping segments merge for a forward seek. An API outage leaves playback
working and shows a notice. The source video is never cut by playback skipping.

Download SponsorBlock settings are separate: selected categories can be marked or
removed with supervised FFmpeg. Native cuts with requested captions require the
fallback to preserve timing. Chapter splitting may round to keyframes. Source and
completed media are preserved when a later processing/publication step fails.

## Downloads, local library and data

```sh
ytui download URL --backend auto
ytui download URL --audio --audio-quality 320K --directory "$HOME/Music/ytui"
ytui download PLAYLIST_URL --playlist --items '1,3:7' --backend yt-dlp
ytui --offline
ytui migrate                          # preview legacy migration
ytui migrate --apply                  # import with backups
```

The queue persists phases, progress, retries, pause/cancel, order, groups and final
output paths. Completion requires a successful process and existing reported final
file. Download options include audio/video quality/container, destination, playlist
ranges, captions, metadata, cover, rate caps and backend. Unknown sizes/ETAs remain
unknown. Native captions support literal language selection and embedding; regex,
translation and additional format conversion remain gaps. Local conversion keeps
its source. Removal/clear deletes records, not completed media.

F2 exposes history/statistics/filtering, page selection, JSON/text import/export,
profile save/load/delete, clipboard review, delayed jobs and settings. Imported
queues never acquire executable hooks from an export. User hooks are opt-in and
receive literal arguments/environment, without a shell. Notification delivery and
Wayland clipboard behavior still need desktop acceptance.

Local resume is explicit with `u`; nothing plays at startup and no history is
uploaded. Offline startup disables account loading, remote providers/thumbnails,
remote playback/jobs and watch polling. It lists existing recorded outputs and
excludes missing/sidecar files. Online queue entries wait for an online session.

Collection watches have a complete initial baseline with no retroactive downloads,
duplicate checks, backoff and account invalidation. Actions adds a watch for the
current collection; the manager pauses/removes/opens it. Limits are two checks at
once, 120 seconds, 32 pages, 2,000 entries and 16 MiB. Download/bandwidth windows use
local HH:MM-HH:MM, including midnight boundaries. Headless `--now` explicitly bypasses
a download window. Long DST transitions still need local acceptance.

F2 → VPN operates on existing NetworkManager VPN/WireGuard profile UUIDs only.
No connection changes happen at startup. This is not a standalone Proton login or
server selector; keep using Proton normally. ytui never requires root for cookies.

State format 2 backs up an old version-1 database before upgrading and rejects
older readers. Migration retains original legacy files, raw unsupported values
and profile/keymap backups. Restore a backup into a separate state directory when
rolling back; do not let an older binary rewrite newer state. The former Python
source remains recoverable from Git history. See [remaining work](REMAINING_WORK.md).

## Validation and diagnostics

`ytui doctor` checks configuration/tools without provider requests. `doctor --network`
probes public YouTube/Music/radio without reading browser sessions or changing VPN.
F2 → Diagnostics reports sanitized recent errors. F2 → Check for updates describes
the source-build update; it never silently replaces binaries. No signed release
archives or distribution packages are implemented yet.

Tests, capture reproduction and actual failures are recorded in
[VALIDATION.md](VALIDATION.md), [FEATURE_AUDIT.md](FEATURE_AUDIT.md) and the
[WQHD gallery](UI_THEMES.md). Browser/keyring, Ghostty graphics, audio devices,
real mpv window integration and successful live YouTube access remain acceptance
gates where the cloud cannot reproduce the user's desktop/network.
