# Shared eager playlist-selection fixtures

`cases.json` supplies 79 bounded ASCII selections and 24 invalid CLI cases.
Each selection has `items` (nullable), legacy `start`/`end`, `reverse`,
materialized `length`, expected zero-based indices and expected warning presence.
Native Rust tests consume the same cases. Two separate source-only Unicode-digit
cases demonstrate accepted upstream grammar that native explicitly rejects.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`:

- `yt_dlp/utils/_utils.py:2418`: `PlaylistEntries`; around 2446 grammar/parser,
  2468 legacy bounds/override warning, 2520 inclusive index/slice iteration.
- `yt_dlp/utils/_utils.py:772`: `orderedSet` keeps first occurrence/request order.
- `yt_dlp/YoutubeDL.py:2091`: playlist processing deduplicates requests before
  reversing the selected order. Around 2150, original `playlist_index` is retained
  while selected-order `playlist_autonumber` starts at one; full `playlist_count`
  and selected `n_entries` remain distinct.
- `yt_dlp/__init__.py:197`, `221`, `451`: positive legacy start, end validation
  and complete explicit item validation, including when items override bounds.
- `yt_dlp/options.py:683` and `1087`: legacy bounds, `-I` and reverse options.
- `test/test_YoutubeDL.py:992`: `test_playlist_items_selection`, including signed
  inclusive ranges, duplicates, negative steps, infinity and clipping phase.

`tools/compare_playlist.py` calls actual `parse_options` with explicit argv
(ignoring user configuration), actual `PlaylistEntries.get_requested_items` on
an eager materialized list, `orderedSet`, then reversal. No network, downloads or
child processes are allowed. The only representation conversion subtracts one
from source one-based indices for the native zero-based API. Order, duplicate
handling, invalid-option acceptance and warning presence are not normalized.

Notable source behavior is preserved:

- Ranges include both endpoints. Negative indices count from the end. Clipping
  preserves the requested step's phase, rather than resetting at index zero or
  the last item. Negative and positive steps are not inferred from bound order.
- Legacy hyphens permit missing bounds (`-`, `--2`, `-:2`). Item `0` is valid but
  selects nothing; legacy `--playlist-start 0` is invalid.
- `inf` becomes positive infinity. The accepted spelling `infinite` instead
  passes through `float_or_none` as `None`. Thus `:inf:-1` selects nothing while
  `:infinite:-1` reverses the entire playlist. The comparison preserves this quirk.
- Explicit empty strings, whitespace and spaces after commas are invalid CLI
  items. The low-level helper alone treats a falsy empty string as absent; native
  construction follows CLI validation instead.
- Valid explicit items override legacy bounds with a warning when those bounds
  are nondefault. Invalid legacy bounds still fail before applying the override.

Native limits remain explicit: ASCII integer grammar, signed 64-bit components,
a 16 KiB item specification and at most 1024 comma-separated segments. Shared
numeric fixtures are small enough to avoid upstream's floating-point end-bound
precision limitations; full exact-i64 endpoint parity is not claimed. Unicode
decimal digits are source-only cases. Lazy/paged acquisition, break-on-existing
archive interactions, playlist randomization and site playlists are excluded.
