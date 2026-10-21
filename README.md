# ytui

A Rust terminal app for YouTube, YouTube Music, downloads and radio. The interface
uses Lazygit-style panes, keyboard navigation, inline thumbnails and mpv playback.
CachyOS + Caelestia + Ghostty + Brave Origin is the primary desktop target.

This is **0.2.0-alpha.4**. Live account and desktop acceptance remains open;
[remaining work](docs/REMAINING_WORK.md) lists unsupported features and service limits.

```bash
git clone https://github.com/Vanir20342222/ytui.git
cd ytui
./install.sh
ytui doctor
ytui
```

The installer builds Rust and installs both `ytui` and the compatible `ytui-rs`
launcher. Install current stable Rust, a C compiler, pkg-config, OpenSSL headers,
mpv and FFmpeg first. On CachyOS/Arch:

```bash
sudo pacman -S --needed base-devel rustup openssl mpv ffmpeg
rustup default stable
```

Until this change reaches main, use the `rewrite/rust-only-settings` branch.
Pulling source alone does not replace an installed binary: rerun `./install.sh`
and check `~/.local/bin/ytui --version`. `install-rust.sh` is a compatibility alias.

- `1`–`4`: Downloads, YouTube, Music, Radio. `b`: feeds; `/`: search; `?`: help.
- `Enter`: play/open; `v`: short muted mpv preview; `d`: download options.
- `,` or `:settings`: general settings window. PgUp/PgDn changes sections;
  F6 focuses categories; arrows edit choices; Space toggles; Enter saves; Esc cancels.
- `:auth`: import a selected local browser session. Personal feeds need sign-in;
  anonymous Home may have no recommendations.
- `u`: explicit local playback resume; `p`: playback queue.
- `ytui --demo`: fictional catalog for UI review; `ytui --offline`: local library.
- `ytui doctor --network`: opt-in public provider diagnostics without importing an account.

Native Rust extraction through pinned [yt-dlp-rs](https://github.com/aagedal/yt-dlp-rs)
is the default. It has incomplete YouTube/challenge/playlist/resume support.
The previously requested **external yt-dlp fallback remains explicit** (`Y` or
`--backend yt-dlp`); ytui never switches to it automatically. It is optional and
is not bundled as a Python app in this repository.

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

Generate WQHD captures entirely in Rust:

```bash
cargo build -p ytui --locked
cargo run -p ytui --example capture_ui --locked -- --binary target/debug/ytui
```

The capture tool runs the actual executable in an isolated PTY with fictional
content. No real account or desktop is captured. A monospaced TTF font is required;
pass `--font /path/to/your/font.ttf` if DejaVu Sans Mono is installed elsewhere.

Licensed under MIT. See [third-party notices](THIRD_PARTY_NOTICES.md).
