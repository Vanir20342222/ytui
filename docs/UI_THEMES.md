# UI themes and WQHD captures

The layout follows Lazygit/Lazydocker's pane-based approach: status, workspaces
and feeds on the left, a large card catalog and smaller details pane on the right,
focused borders, an interactive player strip and optional shortcut hints.
F2 opens Actions; the bottom command entry is removed. `1`–`4` switch workspaces.
On wide screens `b` focuses Feeds, arrows choose, Enter opens and Esc returns.
Small windows use the compact picker. Mouse selection uses the displayed panes.

All images below are **2560×1440 PNGs**, captured from the actual executable's PTY
output at 212×58 cells and rendered with a monospaced font. The compact capture
shows 80×24 cells at a larger font in the same WQHD canvas. These are reproducible
terminal captures, not Ghostty desktop screenshots. Catalogs, credentials and
artwork are fictional; no media or external provider is accessed by the capture.

| Theme | WQHD screenshot | Check |
| --- | --- | --- |
| Catppuccin Mocha | [View](screenshots/themes/catppuccin-mocha.png) | Render, switching, role contrast |
| Catppuccin Latte | [View](screenshots/themes/catppuccin-latte.png) | Render, switching, role contrast; darker muted/status roles |
| Tokyo Night | [View](screenshots/themes/tokyo-night.png) | Render, switching, role contrast |
| Gruvbox | [View](screenshots/themes/gruvbox.png) | Render, switching, role contrast |
| Gruvbox Light | [View](screenshots/themes/gruvbox-light.png) | Render, switching, role contrast |
| Nord | [View](screenshots/themes/nord.png) | Render, switching, role contrast |
| Rose Pine | [View](screenshots/themes/rose-pine.png) | Render, switching, role contrast |
| ytui Dark | [View](screenshots/themes/ytui-dark.png) | Render, switching, role contrast |
| ytui Light | [View](screenshots/themes/ytui-light.png) | Render, switching, role contrast |
| Dracula | [View](screenshots/themes/dracula.png) | Render, switching, role contrast |
| Solarized Dark | [View](screenshots/themes/solarized-dark.png) | Render, switching, role contrast |
| Terminal ANSI | [View](screenshots/themes/terminal.png) | ANSI text/chrome; thumbnail image colors remain intact |
| System preview | [View](screenshots/themes/system.png) | Built-in fallback preview; real desktop following is a separate acceptance check |

Each entry renders all four workspaces and fifteen dialogs at five sizes, including
35×12 and 212×58 (1,235 combinations). All ten download phases are also rendered
in every theme. Built-in RGB text, muted text and selected text must reach contrast
4.5:1; accent/status roles must reach 3:1 against the background. These checks do
not certify arbitrary user palettes, terminal ANSI colors or real desktop themes.

The nine settings categories additionally render their first and last selected
fields in all 13 themes and five sizes: 1,170 settings states. The three download
form categories add 390 states. Keyboard/mouse
editing, invalid-save recovery, cancellation and persistence have separate tests.

Other WQHD views: [Downloads](screenshots/downloads.png),
[YouTube](screenshots/youtube.png), [Music](screenshots/music.png),
[Radio](screenshots/radio.png), [feed focus](screenshots/feeds.png),
[settings](screenshots/settings.png), [account setup](screenshots/auth.png),
[history](screenshots/history.png), [keys](screenshots/keybindings.png),
[offline library](screenshots/offline.png), [resume](screenshots/resume.png),
[watches](screenshots/watches.png), [compact](screenshots/compact.png).

Settings categories: [General](screenshots/settings/general.png),
[Appearance](screenshots/settings/appearance.png),
[Downloads](screenshots/settings/downloads.png),
[Subtitles](screenshots/settings/subtitles.png),
[Network/account](screenshots/settings/network.png),
[Schedule](screenshots/settings/schedule.png), [Tools](screenshots/settings/tools.png),
[Radio](screenshots/settings/radio.png), [Playback](screenshots/settings/playback.png), [compact](screenshots/settings/compact.png).

Recreate captures entirely in Rust:

```sh
cargo build --locked -p ytui
cargo run -p ytui --example capture_ui --locked -- --binary target/debug/ytui
```

The helper isolates XDG/config/state, uses demo and synthetic offline fixtures,
checks every image size, resizes the real terminal and verifies ICANON/ECHO on exit.
The Rust repository test verifies PNG headers and manifest completeness in CI.
Full feature coverage and live acceptance gaps: [FEATURE_AUDIT.md](FEATURE_AUDIT.md).

Alpha.5 adds [Actions](screenshots/actions.png),
[player controls](screenshots/player-controls.png), [search](screenshots/search.png), [categories](screenshots/categories.png),
[download destination](screenshots/download-destination.png), and
[download captions](screenshots/download-captions.png). All 50 captures are WQHD.
The player position/SponsorBlock example is synthetic, not live playback evidence.

Alpha.12 audit panels add [actual formats](screenshots/player-formats.png),
[account setup](screenshots/account-setup.png), [recorded files](screenshots/download-files.png),
[capabilities](screenshots/capabilities.png), [cache](screenshots/thumbnail-cache.png),
[SponsorBlock status](screenshots/sponsorblock-status.png) and
[recent folders](screenshots/recent-folders.png). The full manifest contains 57
WQHD captures. Empty/short dialogs size to their content; the format screenshot
uses explicitly fictional demo metadata. To refresh only these dialogs and diagnostics:

```sh
cargo build -p ytui --locked --example capture_ui
cargo build -p ytui --locked
target/debug/examples/capture_ui --audit-panels-only --binary target/debug/ytui
```
