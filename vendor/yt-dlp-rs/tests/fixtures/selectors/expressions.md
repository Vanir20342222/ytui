# Shared bounded selector expressions

`expressions.json` supplies a native `VideoInfo` dictionary and 156 expressions
with expected ordered selected IDs (one format, two requested formats, or no
match). Four malformed late fallback branches must raise syntax errors during
construction, even though their first branch would otherwise succeed.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`, `YoutubeDL.build_format_selector`
in `yt_dlp/YoutubeDL.py`:

- Around 2390: recursive grouping, merge and fallback parser.
- Around 2540: group, pick-first and merge selector functions.
- Around 2590: best/worst/b/w with video/v/audio/a aliases.
- Around 2640: selector filters constrain the input context before child
  selection. Context flags retain their initial presentation values.

Relevant upstream tests are `test/test_YoutubeDL.py`:
`test_format_selection` (117), `test_format_selection_audio` (148),
`test_format_selection_video` (206), `test_format_selection_string_ops` (268),
and `test_format_filtering` (395). Broader sorting, extension selectors and
multi-stream behaviors exercised there remain outside this fixture.

The reference tool calls actual `build_format_selector` with network access
blocked, format checks disabled and explicit initial context flags derived from
the supplied formats: merged formats are present and the presentation is not
incomplete. No sorter runs. The mixed audio/video/muxed inputs are deliberately
ordered from low to high within each stream kind, agreeing with native
height/bitrate ordering for these cases. Reference-only codec fields match the
native `stream_kind`; native serde ignores those extra fields.

Group filters operate before evaluation: `(bv+ba/b)[height<=720]` removes the
unmeasured audio alternatives, so it picks `mux-high`. In contrast,
`bv[height<=720]+ba` selects `video-mid` and `audio-high`, while
`bv+ba[height<=720]` is unavailable. A missing-inclusive group filter keeps the
audio candidates. Other cases exercise aliases, bare filters, nested groups,
merge/fallback precedence, ordered pair output and unavailable branches.

Comparison preserves selected/requested format IDs and their order. It excludes
native ranking, rendition-group compatibility, merge metadata/container policy,
implicit audio-only fallback differences, star selectors,
extension selectors, commas/all and merges above two streams. No full selector
compatibility claim follows from successful comparison.

Positive `.N` indices are compared for all 20 aliases, using first/second/third
and unavailable positions. Filtering precedes indexed ranking, including group
filters before indexed children; indexed merge/fallback and reverse pair order
are covered. Spellings such as `bv.0`, `bv.01` and `bv.x` remain literal IDs and
are unavailable in this input, rather than syntax errors.

The fixture also verifies a large finite unavailable index and the 64-bit
`isize::MAX` index. Four values above that cap are listed separately in
`excluded_index_cases`: actual pinned upstream evaluation raises `ValueError`
inside `LazyList.__getitem__` (`utils/_utils.py:2277`) when `itertools.islice`
receives a stop above `sys.maxsize`. The reference tool verifies those errors
separately, never converting them to empty selections. Native rejects indices
above `isize::MAX` while parsing the entire expression, including unreachable
fallback branches; that is explicit native policy, not equivalent error timing.
Python's fallback for extremely long integer strings is outside this comparison.

These aligned inputs have no incompatible rendition groups. Native indexed
selectors choose the requested ranked candidate before checking group
compatibility, while native unindexed pairs may search for a compatible pair;
those broader behaviors are tested independently rather than hidden by this
ordered-reference fixture.
