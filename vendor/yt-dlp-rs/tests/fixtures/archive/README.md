# Shared archive identity and membership fixtures

`cases.json` contains full native `VideoInfo` identities, raw UTF-8 archive text,
expected archive IDs/hits and a logical append case. Native archive tests and
`tools/compare_archive.py` consume the same fixtures. Extractor keys lowercase;
IDs retain case, Unicode and internal spaces. The examples include contextual
Greek lowercase, accented/CJK IDs, repeated spaces and numeric-looking IDs.

The raw archive mixes LF, CRLF and lone CR with blank lines, surrounding
whitespace, U+001C/U+001F stripping and unrelated records. These are archive line
loading cases, not permission to use control characters inside identity fields.
The append fixture starts from a file ending in a newline; logical records are
compared rather than platform-specific newline bytes.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`:

- `yt_dlp/YoutubeDL.py:842`: preload with `locked_file` and `line.strip()`.
- `yt_dlp/YoutubeDL.py:1649`: archive check in entry matching before selection.
- `yt_dlp/YoutubeDL.py:1713`: optional early temporary URL-ID archive skip.
- `yt_dlp/YoutubeDL.py:3143`: record after successful per-download archive flags.
- `yt_dlp/YoutubeDL.py:3374`: simulation uses only the explicit force-write flag.
- `yt_dlp/YoutubeDL.py:3674`: archive flag after successful postprocessing/hooks.
- `yt_dlp/YoutubeDL.py:3855`, `3875`, `3883`: identity, membership and recording.
- `yt_dlp/utils/_utils.py:5316`: `make_archive_id`; `1619`: blocking `locked_file`.
- `yt_dlp/options.py:781` and `1322`: archive and force-write options.

The actual reference methods run against a temporary regular file and a set.
Network, downloads and child processes are forbidden. Four actual
`process_video_result` probes verify archived results skip selection/listing,
ordinary simulation/listing do not record, and reference-only forced simulation
can record. Mocked download success/failure is not claimed; native end-to-end
tests independently verify recording follows successful publication.

The following differences remain explicit and are not normalized away:

- Native identities reject empty components, surrounding whitespace and controls.
  Permissive upstream construction and `ie_key`/URL fallback/old IDs are excluded.
- Native extraction completes before archive filtering; no early URL-ID skip is
  implemented, so an archived item can still require page/API/manifest requests.
- Native locks have a bounded five-second wait; upstream lock acquisition blocks.
- Native record rechecks/deduplicates under the lock and repairs a missing final
  newline. Upstream recording appends directly. Duplicate-record and broken-final-
  newline byte behavior is excluded from this comparison.
- Upstream forced archive writing is a reference-only probe; native simulation
  and listing do not opt into it.
