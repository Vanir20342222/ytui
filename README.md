```
  ██╗   ██╗████████╗██╗   ██╗██╗
  ╚██╗ ██╔╝╚══██╔══╝██║   ██║██║
   ╚████╔╝    ██║   ██║   ██║██║
    ╚██╔╝     ██║   ██║   ██║██║
     ██║      ██║   ╚██████╔╝██║
     ╚═╝      ╚═╝    ╚═════╝ ╚═╝
```

# `ytui` — Terminal YouTube & Media Downloader

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Python 3.11+](https://img.shields.io/badge/python-3.11+-brightgreen.svg)](https://www.python.org/)
[![Textual](https://img.shields.io/badge/TUI-Textual-purple.svg)](https://textual.textualize.io/)
[![yt-dlp](https://img.shields.io/badge/engine-yt--dlp-red.svg)](https://github.com/yt-dlp/yt-dlp)

> **ytui** is a modern, full-featured Terminal User Interface (TUI) for downloading, converting, and managing YouTube videos, audio tracks, and playlists directly from your command line. Built with Python 3, `yt-dlp`, `Textual`, and `FFmpeg`.

---

## ✨ Features

- ⚡ **High-Performance Concurrent Downloads**: Parallel downloading engine with dynamic concurrency throttling.
- 🎨 **Modern Centered TUI**: Built with Textual, featuring responsive dark/midnight themes, status badges, and interactive command bars.
- 🔒 **Official Proton VPN Integration**: Dedicated `/vpn` manager panel with server selection (DE, US, Fastest Free Server), real-time connection status monitoring, and account verification.
- 🎵 **FFmpeg Conversion Engine**: Built-in support for converting local or downloaded files into high-quality MP3, AAC, FLAC, M4A, or MP4 formats.
- 🔍 **Interactive YouTube Search**: Type plain text search queries or use `/search <query>` to resolve video metadata before queuing.
- 📋 **Clipboard Autopolling**: Automatically detect copied YouTube URLs and submit them to your queue.
- 💾 **SQLite Persistence & Recovery**: Interrupted or pending downloads automatically recover and resume across app restarts.
- ⌨️ **Vim-Inspired Shortcuts & Slash Commands**: Intuitive `/` command prompt with real-time autocompletion suggestions.

---

## 🚀 Quick Start (1-Command Installation)

You can install `ytui` with a single shell command:

```bash
curl -sSL https://raw.githubusercontent.com/vanir/ytui/main/install.sh | bash
```

Alternatively, clone the repository locally and run the installer:

```bash
git clone https://github.com/vanir/ytui.git
cd ytui
./install.sh
```

Now you can launch the app from **any directory** using just one command:

```bash
ytui
```

---

## 📦 Manual Installation

### Requirements

- **Python 3.11+**
- **FFmpeg** (Recommended for audio extraction & format conversions)
  ```bash
  # Debian / Ubuntu
  sudo apt install ffmpeg

  # Arch Linux
  sudo pacman -S ffmpeg

  # macOS
  brew install ffmpeg
  ```

### Via `pipx` (Recommended)

```bash
pipx install git+https://github.com/vanir/ytui.git
```

### From Source

```bash
git clone https://github.com/vanir/ytui.git
cd ytui
python3 -m venv venv
source venv/bin/activate
pip install -e .
ytui
```

---

## ⌨️ Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `/` or `:` | Focus command input bar |
| `Space` | Pause / Resume selected download item |
| `d` | Delete / Remove selected item from queue |
| `r` | Retry selected failed item |
| `p` | Open **Proton VPN Manager** panel |
| `s` | Open **Settings** panel |
| `?` | Toggle **Help** modal |
| `Ctrl+C` / `q` | Quit application |

---

## 💬 Slash Commands Reference

Type `/` in the command input bar to access interactive commands with autocompletion:

| Command | Description |
| :--- | :--- |
| `/search <query>` | Search YouTube interactively and queue results |
| `/vpn` | Open the Proton VPN Manager panel |
| `/convert [path]` | Open the FFmpeg conversion panel or convert specified local file |
| `/audio` | Switch active mode to Best Audio (MP3) |
| `/video` | Switch active mode to Best Video (MP4) |
| `/subs` / `/subtitles` | Open Subtitle configuration tab |
| `/metadata` | Open Metadata & Tagging configuration tab |
| `/dir` | Open Target Download Directories tab |
| `/keys` | Open Keybindings reference tab |
| `/pauseall` | Pause all active downloads in queue |
| `/resumeall` | Resume all paused downloads in queue |
| `/clear` | Clear completed/finished items from the display queue |
| `/update` | Check for and update the underlying `yt-dlp` engine |
| `/settings` | Open full Application Settings modal |
| `/quit` / `/q` | Exit application safely |

---

## 🛡️ Proton VPN Integration

`ytui` includes a dedicated Proton VPN manager. Simply type `/vpn` or press `p` to open the panel:

- **Server Selection**: Choose between *Fastest Free Server (Auto)*, *Germany (DE)*, *United States (US)*, or custom locations.
- **Smart Free Plan Fallback**: Automatically falls back to available free servers if a country selection requires a Plus subscription.
- **Account Verification**: Displays live login status (`✓ Logged In (user@proton.me)`).

---

## 📄 License

Distributed under the **MIT License**. See [`LICENSE`](LICENSE) for details.
