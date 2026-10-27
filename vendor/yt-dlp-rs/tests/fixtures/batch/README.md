# Saved batch-file reference cases

Pinned official yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.

Each case in `cases.json` has hex-encoded batch-file bytes (`batch_hex`),
positional `urls` and the exact URL list returned by upstream `get_urls` for a
regular batch file (UTF-8 with `errors='ignore'`, universal newlines,
`read_batch_urls` comments/BOMs and final `strip`). The native unit test parses
the same bytes and strips the same positional URLs.

Run `python3 tools/compare_batch.py --upstream /path/to/pinned/yt-dlp`.
