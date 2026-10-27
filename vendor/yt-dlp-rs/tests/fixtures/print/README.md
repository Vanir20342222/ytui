# Saved print reference cases

Pinned official yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.

`cases.json` contains 23 shared cases and one source-only case (a replacement
format spec). Each case has
CLI `args` limited to `-O`/`--print` and `-e`/`--get-id`/`-g`/`--get-filename`,
a native `VideoInfo`-shaped `info`, the prepared `filename`, the
`na_placeholder` and the exact `expected` printed lines (without newlines).
Native tests parse the same arguments and render every shared case; source-only
cases must be rejected natively (when parsed or rendered) and document
upstream's output.

Run `python3 tools/compare_print.py --upstream /path/to/pinned/yt-dlp`. The
worker uses actual `parseOpts` and `YoutubeDL.__forced_printings(...,
incomplete=False)` with socket, download and child-process access blocked.
Lines are compared exactly, including Unicode, JSON escaping and padding.
