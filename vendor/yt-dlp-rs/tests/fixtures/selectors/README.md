# Shared native format predicate cases

`filters.json` contains complete synthetic native format dictionaries and
predicate cases with expected matching IDs in declaration order. Native Rust
predicate tests consume the same cases as `tools/compare_filters.py`.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`:

- `yt_dlp/YoutubeDL.py:2203`, `_build_format_filter`: numeric comparisons,
  optional missing-value inclusion, text operators, negation and quoting.
- `test/test_YoutubeDL.py:395`, `test_format_filtering`: numeric comparisons,
  missing values, decimal/binary units and compound filter examples.
- `yt_dlp/utils/_utils.py:1774`, `parse_filesize`: suffix multipliers, including
  the legacy distinction `k`/`kB` = 1024 and `K`/`KB` = 1000.

The tool calls the actual pinned `_build_format_filter` for every predicate with
network access blocked. No matching IDs, order or predicate text are normalized.
The 113 cases cover all six numeric operators and `?`, the currently supported
numeric/text fields, decimal/binary units, text equality/prefix/suffix/substring
with negation and missing-value inclusion, quotes/backslashes and literal
brackets/slashes/pluses inside quoted values. Additional cases preserve the
numeric-first parsing of unquoted numeric-looking values on text fields (quoted
values compare as strings), optional missing inclusion, and positive/negative
JSON zero rates. Negative zero is a real supplied numeric zero, not missing.

This verifies predicates only. It excludes sorting, ranking, complete selector
grammar, regular expressions and unsupported fields. Filesize inputs and integer
comparison literals are at most `2^53`, where the compared integers are exactly
representable by upstream's numeric-literal float conversion. Native exact-byte
behavior above that limit requires independent native tests; it must not be
claimed as upstream floating-point parity.
