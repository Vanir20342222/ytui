# Saved info JSON reference cases

Pinned official yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.

`cases.json` was generated from actual pinned upstream helpers and is checked by
`python3 tools/compare_info_json.py --upstream /path/to/pinned/yt-dlp`:

- `filenames`: `replace_extension(filename, "info.json", ext)`.
- `playlist_names`: `prepare_filename(ChainMap(playlist, _playlist_infodict(...)),
  "pl_infojson")` with the given template, placeholder and counts. Templates use
  sanitization-neutral values; this is not filename-sanitization parity.
- `sanitize`: `sanitize_info(info, remove_private_keys=True)` with `_version`
  removed (documented rule). Dictionary key order is compared.
- `json_text`: exact `write_json_file` text.
- `source_only`: `_version`, which the native writer omits.

Native unit tests in `src/info_json.rs` consume every shared section.
