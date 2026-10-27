# Saved subtitle embedding plans

Source: official yt-dlp checkout at
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).

The reference runs actual `parse_options`, `FFmpegEmbedSubtitlePP.run`,
`ISO639Utils.short2long`, and `YoutubeDL.run_pp`. Input files exist only in an
in-memory set. `run_ffmpeg_multiple_files` captures argv and creates a virtual
temporary output; `os.replace` and top-level deletions change that set only.
No media contents, real filesystem mutations, network or subprocesses are used.

```sh
python3 tools/compare_subtitle_embedding.py --upstream /path/to/official/yt-dlp
python3 -m unittest discover -s tools -p 'test_compare_subtitle_embedding.py' -v
```

The command checks both the checkout HEAD and fixture pin. Comparisons preserve
exact argv order, inputs, metadata values, warnings/messages, replacements,
planned deletions, actual `run_pp` deletions and file-move bookkeeping. No field
normalization or exclusions apply.

## Corpus contract

`cases.json` contains 57 shared plans, six source-only boundaries, 200 language
projections and one rejected option alias. Each plan provides:

- `args`: explicit parser arguments, independent of user configuration.
- `source`: the media extension and virtual path.
- `subtitles`: ordered language/ext/path/name records, optional `missing`, or
  `null` for absent requested subtitles. These establish eligibility and argv,
  not real subtitle payload parsing.
- Optional `already_have_subtitle`: an explicit public-processor override. Other
  cases use the value produced by actual `parse_options`.
- `expected`: enablement, implied/manual/automatic write flags, retention flag,
  exact captured FFmpeg calls, warnings/messages, PP-returned `delete`, actual
  `deleted_by_run_pp`, virtual `replacements` and `files_to_move`.

All six source containers (`mp4`, `mov`, `m4a`, `webm`, `mkv`, `mka`) have single
and mixed-track cases. Additional cases cover JSON skips, missing paths,
unsupported/uppercase media extensions, empty/absent requests, WebM VTT-only
eligibility, warning deduplication, MP4 ASS warnings, filename helpers, metadata
names, option precedence and retention.

Source-only cases preserve source argv containing newlines or an oversized name,
plus `json3` eligibility outside WebM. Native metadata bounds/control rejection
and a narrower format contract are explicit differences; captured source calls
do not prove real FFmpeg support for `json3`. Two further source-only cases cover
`--compat-options no-keep-subs`, which the native CLI does not implement.

The language table includes all 188 keys in the pinned source's ISO639 map plus
12 longer/unknown/uppercase/empty inputs. Source conversion examines exactly the
first two characters, case-sensitively, and falls back to the original string.
Thus `en-US` and `eng` become `eng`, `fra` becomes `fra`, `sco` surprisingly becomes
`srd` (its `sc` prefix), uppercase `EN` remains `EN`, and empty input remains
empty. This table is source evidence, not an independent standards claim.

## Source details

Embedding maps source streams with `-map 0 -dn -ignore_unknown -c copy`, adds
`-c:s mov_text` for MP4/MOV/M4A, and removes existing subtitle streams using
`-map -0:s`. Every eligible sidecar adds its own `-map N:0` and language metadata.
Truthy names additionally set both `handler_name` and `title`; empty/absent names
add neither. WebM accepts only literal `vtt`; literal `json` is always skipped.
The source emits WebM and MP4 ASS notices at most once per run.

`--embed-subs` alone enables manual subtitle writing, but its generated
`already_have_subtitle` flag remains false because that flag is calculated
*before* implicit writing is enabled. Eligible sidecars are therefore planned
for deletion after embedding. Explicit `--write-subs` sets that flag true and
prevents planned deletion. Automatic-only writing does not set it. An explicit
`--no-write-subs` is still overridden by embedding's implicit manual write when
automatic writing is disabled. Embed/no-embed and write/no-write option ordering
is captured exactly. The apparent `--embed-subtitles` alias is rejected by this
pinned parser and is recorded as an invalid case.

The fixture executes actual `YoutubeDL.run_pp` after the embedding processor.
Without `-k`, it passes planned sidecars to `_delete_downloaded_files`; with `-k`,
it suppresses deletion and registers them in `__files_to_move` instead. This is
why both planned and effective deletion fields are recorded. Ineligible tracks
and skipped/unsupported media never appear in either deletion list. Temporary
media replacement is virtual and captured independently of sidecar retention.

These fixtures establish option, planning and retention agreement only. They do
not establish FFmpeg/container compatibility, media validity, subtitle payload
parsing, real output publication, actual subtitle track contents, or broad
postprocessor parity. Native real-tool tests must establish those separately.
