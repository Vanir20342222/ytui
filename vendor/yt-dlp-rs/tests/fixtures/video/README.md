# Saved video remux/recode plans

Source: official yt-dlp checkout at
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).

`cases.json` records actual `parse_options` results and actual
`FFmpegVideoRemuxerPP.run` / `FFmpegVideoConvertorPP.run` behavior from
`yt_dlp/postprocessor/ffmpeg.py`. The comparison invokes those methods with virtual
input paths and captures their `run_ffmpeg` calls. It never runs FFmpeg, opens a
media file, probes codecs, downloads, or publishes output. The worker forbids
network connections, subprocess creation and filesystem mutation. It overrides
executable discovery so the reference does not depend on installed tools.

Run from the Rust repository root:

```sh
python3 tools/compare_video.py --upstream /path/to/official/yt-dlp
python3 -m unittest discover -s tools -p 'test_compare_video.py' -v
```

The comparator requires that checkout's HEAD and the fixture source pin to match
exactly. It compares every recorded field without normalizing argv, file paths,
case, warnings or deletion lists. Array order matters.

The 70 shared plans cover both modes and all 18 pinned targets: `avi`, `flv`,
`gif`, `mkv`, `mov`, `mp4`, `webm`, `aac`, `aiff`, `alac`, `flac`, `m4a`, `mka`,
`mp3`, `ogg`, `opus`, `vorbis`, and `wav`. They also cover ordered mappings,
unmatched/same-extension skips, source-extension case folding, whitespace in
mappings, nonmatching filename suffixes, repeated flags, selectors, keep/no-keep
precedence, extract-audio combinations and remux/recode conflicts. Eight invalid
CLI forms establish option rejection. One source-only Unicode mapping documents
the native ASCII mapping-source boundary; it is not a native parity claim.

Important source details:

- Remuxing passes `-map 0 -dn -ignore_unknown -c copy`. Recoding passes
  `-map 0 -dn -ignore_unknown`; AVI additionally uses
  `-c:v libxvid -vtag XVID`. The source run method supplies no explicit `-f`
  muxer and generally leaves encoder choice to FFmpeg. Native explicit muxer or
  encoder policies require separate tests and documentation.
- Source extensions are lowercased before mapping and filename substitution.
  Matching the literal filename suffix remains case-sensitive: source `WEBM`
  at `movie.WEBM` converted to MP4 becomes `movie.WEBM.mp4`.
- Same-extension and unmapped cases skip FFmpeg entirely and preserve the
  original `info['ext']` spelling and `info['format']`. The virtual initial
  format string is `fixture`; successful processing changes it to the target.
- A successful processor returns the source filename in `delete`. The top-level
  `keepvideo` flag controls later deletion; it does not change this processor's
  return value. These are planned deletions, not performed file operations.
- `--recode-video` wins over `--remux-video` regardless of command-line order,
  producing the exact recorded warning. `-x` and a video processor are allowed
  together: upstream configures audio extraction before video processing.
  These fixtures capture the video run against the supplied source, not a
  simulated execution of that combined postprocessor chain.

This establishes pure option/planning agreement only. It does not establish
container/codec compatibility, output validity, encoder availability, filesystem
publication behavior, real FFmpeg execution, or broad postprocessor parity.
