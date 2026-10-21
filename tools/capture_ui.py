# /// script
# requires-python = ">=3.11"
# dependencies = ["pyte>=0.8", "Pillow>=10"]
# ///
"""Capture real PTY output at 2560×1440; all content/accounts are synthetic.

Run: uv run --no-project tools/capture_ui.py --binary target/debug/ytui
No production Python dependency. Does not launch media or read browser profiles.
"""
import argparse
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import sqlite3
import struct
import subprocess
import tempfile
import termios
import time
import uuid

import pyte
from PIL import Image, ImageDraw, ImageFont

THEMES = ["system", "catppuccin-mocha", "catppuccin-latte", "tokyo-night",
          "gruvbox", "gruvbox-light", "nord", "rose-pine", "ytui-dark",
          "ytui-light", "dracula", "solarized-dark", "terminal"]
ANSI = dict(zip(["black", "red", "green", "brown", "blue", "magenta", "cyan", "white",
                 "brightblack", "brightred", "brightgreen", "brightbrown", "brightblue",
                 "brightmagenta", "brightcyan", "brightwhite"],
                ["101418", "ff7f86", "8edd99", "f5c36c", "7aa2f7", "cba6f7", "6ddbc1", "e5edf4",
                 "52687b", "ff9ca2", "a6e3a1", "f9e2af", "89b4fa", "e0b6ff", "94e2d5", "ffffff"]))
ANSI.update(yellow=ANSI["brown"], brightyellow=ANSI["brightbrown"])


class Capture:
    def __init__(self, binary, root, offline=False):
        self.master, self.slave = pty.openpty()
        self.before = termios.tcgetattr(self.slave)
        self.screen = pyte.Screen(212, 58)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")()
        self.resize(212, 58, signal_child=False)
        config = root / "config.toml"
        config.write_text('theme="ytui-dark"\nimage_protocol="halfblocks"\nauth_store="file"\nnotifications=false\n')
        env = os.environ.copy()
        env.pop("NO_COLOR", None)
        env.update(TERM="xterm-256color", COLORTERM="truecolor",
                   XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "xdg-state"),
                   XDG_CACHE_HOME=str(root / "cache"))
        self.process = subprocess.Popen([str(binary), "--config", str(config), "--state-dir", str(root / "state"),
                                         "--offline" if offline else "--demo"],
                                        stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                        env=env, start_new_session=True)
        self.read(1.0)
        assert self.process.poll() is None, "startup failed"

    def resize(self, columns, rows, signal_child=True):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
        self.screen.resize(rows, columns)
        if signal_child:
            os.kill(self.process.pid, signal.SIGWINCH)
            self.read(.3)

    def read(self, seconds):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            ready, _, _ = select.select([self.master], [], [], .03)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    break
                if data:
                    self.stream.feed(self.decoder.decode(data))

    def send(self, text, wait=.3):
        os.write(self.master, text.encode())
        self.read(wait)

    def text(self):
        return "\n".join(self.screen.display)

    def shot(self, path):
        deadline = time.monotonic() + 5
        while "Loading thumbnail" in self.text() and time.monotonic() < deadline:
            self.read(.15)
        assert "Loading thumbnail" not in self.text(), "thumbnail fixture did not finish"
        cell_w = min(2544 // self.screen.columns, 1392 // (2 * self.screen.lines))
        cell_h = cell_w * 2
        font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", round(cell_w / .602))
        left = (2560 - self.screen.columns * cell_w) // 2
        top = (1440 - self.screen.lines * cell_h) // 2
        def color(value, background=False):
            if value == "default":
                return "#101418" if background else "#e5edf4"
            return "#" + ANSI.get(value, value if len(value) == 6 else "e5edf4")
        bg = color(self.screen.buffer[0][0].bg, True)
        image = Image.new("RGB", (2560, 1440), bg)
        draw = ImageDraw.Draw(image)
        for y in range(self.screen.lines):
            for x in range(self.screen.columns):
                cell = self.screen.buffer[y][x]
                xx, yy = left + x * cell_w, top + y * cell_h
                draw.rectangle((xx, yy, xx + cell_w - 1, yy + cell_h - 1), fill=color(cell.bg, True))
                if cell.data in ("▀", "▄", "█"):
                    y0 = yy + cell_h // 2 if cell.data == "▄" else yy
                    y1 = yy + cell_h - 1 if cell.data != "▀" else yy + cell_h // 2 - 1
                    draw.rectangle((xx, y0, xx + cell_w - 1, y1), fill=color(cell.fg))
                elif cell.data.strip():
                    draw.text((xx, yy), cell.data, font=font, fill=color(cell.fg))
        path.parent.mkdir(parents=True, exist_ok=True)
        image.save(path)
        assert Image.open(path).size == (2560, 1440)

    def close(self):
        try:
            if self.process.poll() is None:
                self.send("\x1b")
                self.send("q")
                self.process.wait(timeout=5)
            assert self.process.returncode == 0
            mask = termios.ICANON | termios.ECHO
            assert self.before[3] & mask == termios.tcgetattr(self.slave)[3] & mask, "terminal not restored"
        finally:
            if self.process.poll() is None:
                self.process.terminate()
                self.process.wait(timeout=5)
            os.close(self.master)
            os.close(self.slave)


def seed_offline(root):
    state = root / "state"
    state.mkdir(parents=True)
    media = root / "Studio Sessions.mp3"
    media.write_bytes(b"synthetic fixture; never played")
    item = dict(id="local-fixture", title="Studio Sessions · A quiet evening", creator="Fictional catalog",
                url=str(media), kind="song", duration=240, thumbnail="")
    job = dict(id=str(uuid.uuid4()), generation=0, media=item, options=dict(audio=True), state="Done",
               progress=dict(downloaded=0, total=None, speed=None, eta=None), outputs=[str(media)], error=None, attempt=0)
    snapshot = dict(downloads=dict(jobs=[job]), positions=dict(entries=[dict(media=item, seconds=82, duration=240,
                    updated_at=1, finished=False)]), watches=[dict(id=str(uuid.uuid4()), title="Studio Sessions · New releases",
                    request=dict(Collection="VLfixture"), music=True, options=dict(audio=True), interval_seconds=1800,
                    next_check=0, enabled=True, generation=0, owner="anonymous", seen=None, failures=0, error=None)])
    with sqlite3.connect(state / "state.db") as db:
        db.executescript("CREATE TABLE app_state(name TEXT PRIMARY KEY,data TEXT NOT NULL); PRAGMA user_version=2;")
        db.execute("INSERT INTO app_state VALUES('snapshot',?)", (json.dumps(snapshot),))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/debug/ytui"))
    parser.add_argument("--output", type=Path, default=Path("docs/screenshots"))
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="ytui-wqhd-") as directory:
        root = Path(directory)
        demo = root / "demo"
        demo.mkdir()
        capture = Capture(args.binary.resolve(), demo)
        try:
            assert "Workspaces" in capture.text()
            for theme in THEMES:
                capture.send(f":theme {theme}\r", .5)
                assert theme in capture.text(), f"theme not applied: {theme}"
                capture.shot(args.output / "themes" / f"{theme}.png")
            capture.send(":theme ytui-dark\r", .5)
            for tab, name in [(1, "downloads"), (2, "youtube"), (3, "music"), (4, "radio")]:
                capture.send(str(tab), .5)
                capture.shot(args.output / f"{name}.png")
            capture.send("2b")
            assert "Enter opens" in capture.text()
            capture.shot(args.output / "feeds.png")
            capture.send("\x1b")
            for command, name in [("stats", "history"), ("keys", "keybindings"), ("settings", "settings"), ("auth", "auth")]:
                capture.send(f":{command}\r")
                capture.shot(args.output / f"{name}.png")
                capture.send("\x1b")
            capture.send("/qvds")
            assert capture.process.poll() is None
            capture.send("\x1b")
            capture.resize(80, 24)
            capture.shot(args.output / "compact.png")
            capture.resize(35, 12)
            capture.send("b")
            assert "Browse feeds" in capture.text()
            capture.send("\x1b")
            capture.resize(212, 58)
        finally:
            capture.close()
        offline = root / "offline"
        seed_offline(offline)
        capture = Capture(args.binary.resolve(), offline, offline=True)
        try:
            assert "Offline downloads" in capture.text()
            capture.shot(args.output / "offline.png")
            for command, name, expected in [("resume", "resume", "1:22"), ("watches", "watches", "New releases")]:
                capture.send(f":{command}\r")
                assert expected in capture.text()
                capture.shot(args.output / f"{name}.png")
                capture.send("\x1b")
            capture.send("/quiet\r")
            assert "Studio Sessions" in capture.text()
        finally:
            capture.close()
    shutil.copyfile(args.output / "themes" / "ytui-light.png", args.output / "light.png")
    files = []
    for path in sorted(args.output.rglob("*.png")):
        assert Image.open(path).size == (2560, 1440), f"Not WQHD: {path}"
        files.append(str(path.relative_to(args.output)))
    (args.output / "manifest.json").write_text(json.dumps(dict(
        source="Actual Rust executable PTY output; fictional demo/offline data and generated artwork",
        binary=str(args.binary), size=[2560, 1440], terminal=[212, 58], compact_terminal=[80, 24],
        themes=THEMES, captures=files, real_desktop_capture=False), indent=2) + "\n")
    print(json.dumps(dict(themes=len(THEMES), size="2560x1440", resize="212x58 → 80x24 → 35x12 → 212x58",
                          demo_and_offline=True, terminal_restored=True, output=str(args.output))))


if __name__ == "__main__":
    main()
