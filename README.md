# ytui

A Rust terminal app for YouTube, YouTube Music, downloads and radio. The interface
uses Lazygit-style panes, keyboard navigation, inline thumbnails and mpv playback.
CachyOS + Caelestia + Ghostty + Brave Origin is the primary desktop target.

This is **0.2.0-alpha.7**. Live account and desktop acceptance remains open;
[remaining work](docs/REMAINING_WORK.md) lists unsupported features and service limits.

```bash
git clone --branch fix/playback-fallback https://github.com/Vanir20342222/ytui.git
cd ytui
./install.sh
ytui doctor
ytui
```

The installer builds Rust and installs both `ytui` and the compatible `ytui-rs`
launcher. Install current stable Rust, a C compiler, pkg-config, OpenSSL headers,
mpv and FFmpeg first. On CachyOS/Arch:

```bash
sudo pacman -S --needed base-devel rustup openssl mpv ffmpeg yt-dlp deno
rustup default stable
```

Until this change reaches main, use the `fix/playback-fallback` branch.
Pulling source alone does not replace an installed binary: rerun `./install.sh`
and check `~/.local/bin/ytui --version`. `install-rust.sh` is a compatibility alias.

- `1`–`4`: Downloads, YouTube, Music, Radio. `b`: feeds; `/`: search; `?`: help.
- `Enter`: play/open; `v`: short muted mpv preview; `d`: download options.
- `,`: general settings window; `F2`: searchable Actions; `F1`: toggle bottom hints. PgUp/PgDn changes sections;
  F6 focuses categories; arrows edit choices; Space toggles; Enter saves; Esc cancels.
- `F2 → Account`: import a selected local browser session. Personal feeds need sign-in;
  anonymous Home may have no recommendations.
- `u`: explicit local playback resume; `p`: playback queue.
- `ytui --demo`: fictional catalog for UI review; `ytui --offline`: local library.
- `ytui doctor --network`: opt-in public provider diagnostics without importing an account.
- `ytui playback-check URL --backend auto`: check three seconds of actual mpv playback without browser cookies.

The recommended `auto` backend tries pinned [yt-dlp-rs](https://github.com/aagedal/yt-dlp-rs)
first and falls back to external yt-dlp for supported extraction limitations.
Strict `native` and explicit `yt-dlp` modes remain available. Existing settings keep
their chosen mode: **F2 → Select extraction backend → auto** enables automatic
fallback after upgrading. The application stays Rust; external yt-dlp is separately
installed. Successful live YouTube access still depends on the account/network.

Playlist links select collection downloads automatically; watch links with playlist
context stay single-video unless explicitly expanded. Playlist jobs retain their
source across entry metadata/retries, and auto uses yt-dlp for native playlist limits.
[Playback/playlist fixes and validation boundaries](docs/PLAYBACK_REPAIR.md).

Thumbnails use sized memory/disk caches and nearby prefetch. Feeds load continuation
pages near the end and recover temporary errors automatically, keeping visible
videos and selection. Fresh HTML Home data saves an extra request; overlapping
pages no longer end the feed immediately. Server categories/section labels remain.
Wide cards
get most of the width, with square Music art. `o` opens player controls; **g in mpv**
opens its window menu. Volume/mute persist. `B` toggles SponsorBlock skipping and `U`
replays a skipped part; Settings → Playback controls categories and skip/mark/off.

Rust migration reads existing legacy settings, profiles and queues on request:
`ytui migrate` previews; `ytui migrate --apply` imports with backups. Existing
user files and downloaded media are retained. The old Python source is available
in Git history.

[Usage and settings](docs/RUST.md) · [Remaining work](docs/REMAINING_WORK.md) ·
[Feature audit](docs/FEATURE_AUDIT.md) · [Validation](docs/VALIDATION.md) ·
[WQHD theme gallery](docs/UI_THEMES.md) · [Rewrite plan](RUST_REWRITE_PLAN.md)

![Rust terminal demo with fictional content](docs/screenshots/youtube.png)

Run checks with `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and
`cargo test --workspace --locked`. The separate real-media fixture and opt-in
live YouTube tests are described in the validation record.
Run `tools/test-youtube.sh auto` for isolated public browsing, playback and
two-entry playlist checks; [live test instructions](docs/LIVE_YOUTUBE_TESTS.md)
describe the required tools, optional public fixture URLs and CI runner.

Generate WQHD captures entirely in Rust:

```bash
cargo build -p ytui --locked
cargo run -p ytui --example capture_ui --locked -- --binary target/debug/ytui
```

The capture tool runs the actual executable in an isolated PTY with fictional
content. No real account or desktop is captured. A monospaced TTF font is required;
pass `--font /path/to/your/font.ttf` if DejaVu Sans Mono is installed elsewhere.

Licensed under MIT. See [third-party notices](THIRD_PARTY_NOTICES.md).
