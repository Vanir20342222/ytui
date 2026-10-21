# UI themes and WQHD captures

The layout follows Lazygit/Lazydocker's pane-based approach: status, workspaces
and feeds on the left, a dense catalog and large detail/preview pane on the right,
focused borders, a player strip and a short command bar. `1`–`4` switch workspaces.
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

Each entry renders all four workspaces and twelve dialogs at five sizes, including
35×12 and 212×58 (1,040 combinations). All ten download phases are also rendered
in every theme. Built-in RGB text, muted text and selected text must reach contrast
4.5:1; accent/status roles must reach 3:1 against the background. These checks do
not certify arbitrary user palettes, terminal ANSI colors or real desktop themes.

Other WQHD views: [Downloads](screenshots/downloads.png),
[YouTube](screenshots/youtube.png), [Music](screenshots/music.png),
[Radio](screenshots/radio.png), [feed focus](screenshots/feeds.png),
[settings](screenshots/settings.png), [account setup](screenshots/auth.png),
[history](screenshots/history.png), [keys](screenshots/keybindings.png),
[offline library](screenshots/offline.png), [resume](screenshots/resume.png),
[watches](screenshots/watches.png), [compact](screenshots/compact.png).

Recreate captures with development-only Python helpers (the application remains Rust):

```sh
cargo build --locked -p ytui
uv run --no-project tools/capture_ui.py --binary target/debug/ytui
```

The helper isolates XDG/config/state, uses demo and synthetic offline fixtures,
checks every image size, resizes the real terminal and verifies ICANON/ECHO on exit.
Full feature coverage and live acceptance gaps: [FEATURE_AUDIT.md](FEATURE_AUDIT.md).
