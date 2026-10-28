# Saved subtitle conversion plans and exact TTML text

Source: official yt-dlp checkout at
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).

The corpus exercises actual `parse_options`,
`FFmpegSubtitlesConvertorPP.run` from `yt_dlp/postprocessor/ffmpeg.py`, and
`dfxp2srt` from `yt_dlp/utils/_utils.py`. Four TTML inputs are copied exactly
from `test/test_utils.py::test_dfxp2srt`, including its original expected SRT
assertions: ordinary text/timing, missing default namespace, forward/inherited
styles, and UTF-16 with a BOM. Each corpus entry records its provenance.

Run from the Rust repository root:

```sh
python3 tools/compare_subtitle_conversion.py --upstream /path/to/official/yt-dlp
python3 -m unittest discover -s tools -p 'test_compare_subtitle_conversion.py' -v
```

The tool checks the source checkout HEAD and fixture pin before executing the
worker. All fields are compared exactly, including text, timestamps, trailing
newlines, path/extension spelling, ordered calls/deletions, messages, warnings,
and file-move metadata. No volatile-field exclusions or text normalization apply.

## Corpus contract

`cases.json` contains:

- `ttml_cases`: 15 byte-preserving input files with `expected_srt` text. These
  include the four upstream inputs plus timing offsets, colon/fractional clocks,
  millisecond boundaries, invalid timing/index gaps, zero-end/duration fallback,
  reversed ranges, all-invalid empty output, inline text/entities/whitespace,
  cross-cue style state, unknown styles, legacy namespaces, UTF-16BE with a BOM,
  and UTF-8 incorrectly marked with the source's supported single-quoted
  `encoding='UTF-16'` workaround. Read input files as bytes; UTF-16 inputs are
  intentionally not UTF-8.
- `cases`: 42 parser/postprocessor plans, covering all ordinary source/target
  combinations among SRT/VTT/ASS/LRC, the DFXP/TTML/TT native-SRT path, aliases,
  `none`, repeated flags, missing/empty/absent subtitle requests, literal JSON
  skips, source filename suffix replacement, mixed declaration order,
  `--skip-download`, and audio/video processor combinations.
- `source_only_cases`: the literal `json3` extension is not upstream's special
  `json` skip. Its captured source plan attempts FFmpeg; native unsupported
  source-format rejection is recorded explicitly. This plan does not prove that
  FFmpeg can parse such input.
- `invalid_cases`: five rejected CLI targets/forms, including uppercase and
  unsupported mapping syntax.

For plan cases, `subtitles` supplies ordered language/metadata records and
virtual source file contents (`file_text` or `input_file`). `expected` records
selected target/processor stage, selector and skip-download state, captured
FFmpeg argv, exact warnings/messages, updated requested metadata, planned
source/intermediate deletion paths, native TTML writes, and `__files_to_move`.
FFmpeg output is deliberately synthetic `FFMPEG:FORMAT\n`; it tests propagation
into requested metadata and never represents real converted subtitle text.

## Source behavior and evidence limits

The postprocessor runs `before_dl`: the source downloads selected sidecars,
executes this processor, and only then branches on `skip_download` in
`YoutubeDL.py`. Consequently `--skip-download` still permits subtitle conversion.
The option alone does not select subtitles for writing.

The target list is exactly `srt`, `vtt`, `ass`, `lrc`; lowercase `none` disables
the processor. Both `--convert-sub` and `--convert-subtitles` alias
`--convert-subs`, and the last value wins. A VTT target uses the FFmpeg muxer
name `webvtt`. Source same-format and literal JSON cases skip conversion.

DFXP/TTML/TT is first converted by actual `dfxp2srt` to a virtual SRT file and
emits the exact style-loss warning. For an SRT target, upstream immediately
continues without adding the SRT destination to `__files_to_move`; the fixture
preserves that observable bookkeeping quirk. Other targets capture the second
SRT-to-target FFmpeg step and both planned deletion paths.

The worker reads only the saved input files before virtualizing postprocessor
file access. It captures executable calls instead of running them, supplies
virtual existence checks and in-memory reads/writes, and forbids network,
process creation and filesystem mutation. Tests deliberately inject hostile
postprocessor actions to verify these guards. No actual media/download/filesystem
publication or real FFmpeg conversion is exercised by this reference. Exact
TTML outputs establish the saved text subset; they do not establish general
TTML conformance, frame/tick timing, arbitrary encodings, unbounded styles, or
all subtitle format behavior.
