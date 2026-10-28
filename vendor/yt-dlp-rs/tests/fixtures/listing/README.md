# Shared format-listing metadata and reference flow probes

`input.json` is an unselected native `VideoInfo` with four synthetic adaptive-only
formats. Reference-only codec fields agree with native `stream_kind`; native
serde ignores those extra fields. `expected.json` contains declaration IDs and
an explicit metadata allowlist: ID, stream kind, extension, width/height,
filesize, total bitrate, language and note. Renderer tests consume the same
input and metadata. Missing optional values remain null.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`:

- `yt_dlp/options.py:945`: `-F`/`--list-formats` option.
- `yt_dlp/YoutubeDL.py:786`: unset simulation becomes `list_only` for listing.
- `yt_dlp/YoutubeDL.py:3048`: list before selection; `list_only` returns after
  forced printing, without selecting formats.
- `yt_dlp/YoutubeDL.py:3982` and `4080`: actual format table renderer/list method.
- `yt_dlp/__init__.py:755` and `817`: JSON/getting options imply simulation.

`tools/compare_listing.py` calls the actual pinned `list_formats` and table
renderer directly, observing the declaration-order input and metadata. The
native renderer now follows upstream `render_formats_table`; exact rendered
bytes are pinned separately by `table.txt` (below).

The same tool separately runs actual `parse_options` and `process_video_result`
with network, download and external processes forbidden. Its eight probes show:

- `-F`, `-F -f v360` and `-F -f unavailable` list all formats and return unselected.
- `-F -j -f v720+a128` and `-F -s -f v720+a128` select that explicit pair, then
  simulate; JSON mode prints selected JSON.
- Reference-only `-F --print-json` uses `list_only` and prints unselected JSON.
  Native does not implement `--print-json`.
- `-F -j -f unavailable` prints the table before reporting selection failure.
  Native validates the whole input before printing its table; atomic output is
  an explicit native policy difference.
- Malformed `-f bv+` fails construction before listing.

Actual processing sorts formats, so control-flow probes compare listed ID sets,
not native declaration order. Explicit pair IDs avoid claiming parity for the
native default selection, ranking, group rules or FFmpeg discovery. All probes
exercise synthetic inputs only; no acquisition or live-site behavior is claimed.

`table.txt` is the exact stdout of the pinned upstream for `input.json` (with
the native-only `stream_kind`/`direct` keys removed), produced by
`YoutubeDL({'quiet': True, 'listformats': True}).process_ie_result(info, download=False)`.
The native renderer must reproduce it byte for byte; the `[info] Available
formats for ID:` caption is a screen message and is not part of it.
