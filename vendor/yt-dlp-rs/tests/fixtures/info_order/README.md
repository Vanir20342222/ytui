# Saved upstream `-j` lines (info dict key order)

Pinned official yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.

`site/` holds the served files; `cases.json` the exact `-j --user-agent fixture`
lines (`lines`) and `-J` lines (`single`) pinned upstream printed for each, with the `Content-Type` and
`Last-Modified` headers it was served with. The origin is `{ORIGIN}` (its host
`{HOST}`), and the volatile `epoch` and `_version` (upstream's release) are
removed; nothing else is normalized.

The cases cover GenericIE's typed direct link (`clip.mp4`), its untyped
"URL could be a direct video link" result (`clip.qqq`), a single HTML5 embed
(`one.html`), one with a poster, subtitle tracks and Open Graph metadata
(`rich.html`), an HTML5 playlist with a `<source>` (`three.html`), an HLS
master with subtitle renditions, a typed and an untyped (`#EXTM3U` body,
`text/plain`) media playlist, and DASH manifests with segmented and direct text
tracks.

Regenerate with `python3 tools/capture_info_order.py --upstream PATH --write`
(without `--write` it checks the saved lines). `tests/info_order.rs` serves the
same files and headers to the native CLI and compares its lines exactly.

`loaded/` holds info JSON written by the pinned upstream (`--skip-download
--write-info-json --user-agent UA`, host replaced by `media.invalid`,
`epoch` fixed) and upstream's `--load-info-json` `-j`/`-J` lines for it
(`epoch` and `_version` removed).
