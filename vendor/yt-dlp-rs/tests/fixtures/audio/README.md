# Saved extract-audio postprocessor plans

Official pinned yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.
Run `python3 tools/compare_audio.py --upstream /path/to/pinned/yt-dlp`.

`cases.json` contains 73 supported source plans, five explicitly source-only
cases and four actual source option rejections. Each case supplies CLI `args`
and `source` filepath/ext/fixed probed codec. `expected` contains the actual
selector option, mapping target, resulting extension/path, captured ffmpeg
calls (`input`, `output`, `codec`, `more_opts`), requested deletion paths,
virtual rename operations, probe calls and a numeric quality argument projection.

The reference uses actual parse_options and FFmpegExtractAudioPP.run. Its tiny
subclass replaces only codec probing and ffmpeg execution with deterministic
capture; os.replace is replaced with a recorder. The actual mapping resolver,
quality calculation, branch choices, output naming and information updates are
executed. FDK is explicitly unavailable, matching native's ordinary AAC encoder.
Virtual relative paths do not represent real media, and no files are renamed or
created. Socket access and subprocesses are forbidden. No Python postprocessor
is used by native runtime code.

Actual source call arrays are retained without string normalization. Native
plan tests can compare supported codecs/options and map `-f adts` to a typed
muxer choice. They compare numerical quality option values through
`quality_args`: e.g. `128.0k` and `128k` represent the same numeric bitrate and
unit, but their source spelling is still recorded in `more_opts`. This sole
explicit quality projection does not discard encoder, bitrate, unit, muxer or
path differences. Native explicit muxer selection versus source extension-based
inference is not a claim of byte-identical ffmpeg commands or output files.

Cases cover best/mp3/aac/m4a/opus/vorbis/flac/wav, same-codec copies, common-audio
best skips, unknown-codec best fallback to MP3, ordered A>B/C mappings and
no-match skip. ALAC source plans are explicitly source-only because native uses
a real ALAC encoder instead of duplicating upstream's option-loss bug. Curves
cover 0/5/10, fractional quality, 128K/128k and fractional bitrates. Above 10 is
bitrate k-units even when fractional; at/below 10, encoder-specific VBR curves
apply only to supported encoders. Opus/flac/wav do not acquire an invented VBR
quality argument. Source qualities are converted to float first.

Source AAC quirks are preserved: target aac uses the source's m4a extension;
AAC copy with that target retains `-f adts`. AAC encoding loses ACODECS' adts
options when source replaces more_opts with quality options, so actual output
container inference follows m4a. The fixture records both rather than changing
extensions to a nicer target name. m4a AAC copy retains aac_adtstoasc.

Same-extension transcode uses movie.temp.ext and returns movie.orig.ext for
later deletion; same-extension copy skips execution. Mismatching final filename
extensions cause extension append, not removal of unrelated suffixes. Returned
deletion paths are postprocessor results: --keep-video prevents deletion in
outer orchestration, not this run method. Atomic native publication/retention
and real ffmpeg behavior are tested separately.

Actual option evaluation verifies -x defaults to bestaudio/best only if an
explicit selector and --keep-video are absent; with --keep-video the upstream
format option remains null. Source validates formats/mappings and negative
quality. It unexpectedly accepts bad/inf/nan quality strings because option
validation substitutes integer zero; actual preferred-quality conversion then
omits quality or emits nonfinite arguments. Native rejects those inputs and
bounds finite quality to its documented range. Nonfinite source-only numeric
projection is a string so the shared JSON remains standards-compliant.

Source ALAC encoding selects acodec=None and loses the table's `-acodec alac`
when replacing options with quality args, leaving ffmpeg's default encoder. The
matching-codec ALAC copy path keeps a later `-acodec alac` after copy, overriding
that copy command. Both actual plans are retained as source-only evidence.
Native intentionally selects ALAC/copy directly rather than silently producing
AAC or introducing an unintended re-encode.

Source references:

- `yt_dlp/postprocessor/ffmpeg.py:50-61`: ACODECS extension/encoder/options.
- `yt_dlp/postprocessor/ffmpeg.py:67-80`: ordered resolve_mapping.
- `yt_dlp/postprocessor/ffmpeg.py:432-535`: FFmpegExtractAudioPP, common-audio
  skips, float quality curves, codec/copy fallback, option replacement,
  temp/original naming and return/update behavior.
- `yt_dlp/__init__.py:258,263-266`: audio format and quality validation;
  `594-596`: extract-audio default selector; `656-662`: postprocessor options.
- `yt_dlp/options.py:1617-1634`: extract-audio/format/quality options;
  `1667-1671`: keep-video option and retained intermediate file policy.

These components also implement probing, subprocess execution, filesystem
mutation, progress, timestamp changes and many other postprocessors. Those are
outside this deterministic planner comparison; no audio payload quality,
container correctness, tool discovery or runtime fallback claim is made.
