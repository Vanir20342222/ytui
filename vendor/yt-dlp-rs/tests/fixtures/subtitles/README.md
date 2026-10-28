# Saved HTML5 subtitle references

Official pinned source: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.
Run `python3 tools/compare_subtitles.py --upstream /path/to/pinned/yt-dlp`.
The worker invokes actual upstream methods and blocks sockets, child processes,
downloads, webpage acquisition and manifest extraction. Direct MP4/MP3 sources
ensure the HTML parser never needs a media request.

`cases.json` has seven HTML metadata cases, 29 subtitle selection cases, five
portable sidecar filename cases, six option combinations and three explicitly
source-only cases. The expected objects are saved results of the actual source,
not calculations from native behavior. The supported metadata/selection fixtures are shared with native tests; option
cases establish source flag semantics only.

Language dictionaries are represented as ordered arrays of `{language,formats}`
entries, preserving `None` as JSON null independently of the literal string
`null`, empty labels and declaration order. The source HTML parser adds only
URLs. The reference applies actual `sanitize_url(url)` and then
`determine_ext(url).lower()`, matching `process_video_result` postprocessing.
A protocol-relative source becomes `http://` even with an HTTPS page base; the
actual sanitizer repairs `httpss://` to `https://`. These schemes are preserved
exactly in the expected output rather than ignored for comparison. Extensionless URLs use the actual
`unknown_video` default. These are actual source processing steps, not comparison normalization; no
scheme, language, filename or order differences are ignored. The supplied `page_url` is the
parser's effective base, not evidence of upstream HTML `<base>` handling.

Source HTML permits absent/empty `kind`, or exact lowercase `subtitles` and
`captions`; uppercase variants are excluded. It strips `src`, expands entities
through attribute parsing and joins URLs. `srclang or lang or label` preserves
whitespace/case in nonempty attributes and preserves a final empty `label` as
an empty language. Missing all language attributes produces None. Native direct
child DOM ownership is stricter than source regex matching; nested media,
malformed HTML and extractor-specific DOM behavior are outside this comparison.

Selection cases invoke `YoutubeDL.process_subtitles` with normal subtitles,
`writesubtitles` and actual language/format options. Defaults prefer exact `en`,
then a case-sensitive `en` prefix, then first declared. Literal selections are
case-insensitive full matches. `all`, removals and re-additions retain actual
order; absent literals select nothing without a warning. Formats use ordered
slash preferences, selecting the last matching format, or last available with a
warning if none match. Native supports literals/aliases rather than general
regex; the `en.*` source-only case verifies the difference. None-language source
failures are captured as actual `AttributeError`, and never relabeled `und`.
Native explicitly rejects requested unknown languages. Automatic captions,
conversion, embed, overwrite and unsafe sidecar paths are outside this slice.
Portable filename cases call actual `subtitles_filename`; no cross-platform
unsafe filename or byte-for-byte table claim is made.

Relevant source locations:

- `extractor/common.py:3351-3491`: `_parse_html5_media_entries`, track ownership,
  kind, URL and language metadata.
- `YoutubeDL.py:2899-2919`: URL sanitation, missing ext completion and subtitle selection before
  format selection; `3157-3220`: `process_subtitles` language/format selection.
- `utils/_utils.py:1322-1323`: `subtitles_filename`; `5329-5360`: ordered language
  options and case-insensitive full-match regex behavior.
- `options.py:973-1008`: write/list/language/format options;
  `YoutubeDL.py:788-791`: list-only simulation default. Actual option probes
  verify `--list-subs` alone and with `--write-subs` use `list_only`; `-j` or `-s`
  changes that to true simulation, with format selection retained. Listing is
  performed before selection at `3052-3062`, as for `-F`.
- `YoutubeDL.py:3373-3376` simulation returns before sidecar writes;
  `3389` writes requested subtitles before `3460-3464` skip-download handling.
  Therefore `--skip-download --write-subs` writes sidecars without the media,
  whereas simulation/list-only does not. Plain skip-download sets archive-write
  to `force_write_download_archive` (normally absent), not true. Successful media
  sets true at `3674`; archive aggregation/recording is `3143-3146`.
- `YoutubeDL.py:4447-4505`: sidecar filenames, existing-file handling and download
  failures. These transport/timing details are source research; the comparison
  never acquires or writes subtitle payloads.
