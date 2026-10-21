"""CI-only PNG header check; no image libraries or runtime application dependency."""
from pathlib import Path
import struct

paths = sorted(Path("docs/screenshots").rglob("*.png"))
assert paths, "No screenshots found"
for path in paths:
    with path.open("rb") as source:
        header = source.read(24)
    assert header[:8] == b"\x89PNG\r\n\x1a\n", f"Not a PNG: {path}"
    assert struct.unpack(">II", header[16:24]) == (2560, 1440), f"Not WQHD: {path}"
print(f"{len(paths)} screenshots verified at 2560x1440")
