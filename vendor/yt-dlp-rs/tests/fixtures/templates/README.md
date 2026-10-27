# Saved output-template reference cases

Pinned official yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.

`cases.json` contains 213 shared cases and three explicitly source-only cases.
Each case has a complete native `VideoInfo`-shaped `info`, a `template`, the
`na_placeholder` string and an exact `expected` string. Native tests consume
`cases`; `source_only_cases` document unsupported behavior without making a
native parity claim. The hidden input `__last_playlist_index` supplies padding
context and is not a public rendered field.

Run `python3 tools/compare_templates.py --upstream /path/to/pinned/yt-dlp`.
The worker invokes actual `YoutubeDL.evaluate_outtmpl(..., sanitize=True)`.
It blocks socket creation, downloads, URL acquisition and child processes.
No rendered output is trimmed, padded, renamed, normalized or recomputed by a
local imitation of Python formatting. Unicode and intentional formatting spaces
are compared exactly. Source values/defaults in shared cases avoid differences
between upstream filename sanitization and native `safe_component`; URL fields
exist in the input but raw URL substitution is source-only. This is metadata
formatting evidence, not filename sanitization or filesystem-path parity.

Covered conversions are `s`, `d`, `i`, `u`, `f`, `F`, `e`, `E`, `g`, `G`, `c`,
`r`, `a`, `l`, `j`, `h`, `q`, `B`, `U`, `D` and `S`; key maths, negation, `>`
datetime formatting, `&` replacements and alternate chains; a case may set
`compat_opts` (`filename-sanitization`); Python flags `#0-+ `,
width, precision and inert `h/l/L` modifiers; known fields, ordered alternatives,
explicit defaults (including empty defaults) and custom global placeholders.
Missing/non-numeric numeric values discard width/precision/flags. An available
non-numeric value does not cause the next alternate to be tried. Empty string
values become unavailable under the actual filename sanitizer. Numeric conversion
first calls `float_or_none`, including integer metadata, so filesize
9007199254740993 renders as 9007199254740992 and u64::MAX renders as
18446744073709551616 with `d`. These source rounding effects are preserved.
ASCII numeric strings cover fractional truncation, negative zero, ties-to-even,
underscores and nonfinite float spellings. Nonfinite integer conversions raise
source errors and are not represented as successful shared cases.

Exact bare `s` on integer `playlist_index` auto-pads to the digits of
`__last_playlist_index` (or zero); `playlist_autonumber` uses `n_entries`.
Explicit width or inert modifiers disable this bare-format compatibility rule.
The ordinary `autonumber` field depends on downloader state, is not represented
by native `VideoInfo` and is source-only.

Source references:

- `yt_dlp/YoutubeDL.py:1267-1522`: `prepare_outtmpl` and `evaluate_outtmpl`,
  field compatibility widths, alternatives/defaults, missing and numeric casts.
- `yt_dlp/utils/_utils.py:2885-2900`: `STR_FORMAT_RE_TMPL` and Python printf
  flags/width/precision/length syntax.
- `yt_dlp/options.py:1420-1423`: `--output-na-placeholder`, default `NA`, parameter
  `outtmpl_na_placeholder`.
- `test/test_YoutubeDL.py:622-758`: saved output metadata, filename/template
  evaluations, escaped percent, padding, missing placeholders and numeric casts.

These mapped files also contain unrestricted arbitrary metadata. Unknown fields
and keys that do not match `INTERNAL_FORMAT_RE` are missing values, as upstream;
malformed/raw percent forms are native errors. Source-only cases
make selected differences explicit, without claiming those unsupported features.
Native templates remain bounded to 16 KiB, 256 percent tokens, width/precision
4096 and expanded output 64 KiB; upstream has no equivalent resource contract.
Python accepts additional numeric string syntax such as Unicode decimal digits;
these cases establish the shared ASCII numeric-string subset only. Native
filename sanitation and error/path behavior have independent Rust tests.
