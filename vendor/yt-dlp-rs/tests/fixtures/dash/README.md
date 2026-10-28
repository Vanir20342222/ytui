# Original static DASH fixtures

`plan-number.mpd`, `plan-time.mpd` and `plan-list.mpd` are handwritten planner
fixtures. They describe synthetic URLs and ranges, not media. The comparison tool
documents which fields match the pinned upstream parser and independently checks
the ranges that upstream ignores.

`media/` contains original blue video and sine audio generated offline with
FFmpeg 9.0.2. No third-party media is included. From the repository root:

```sh
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i color=c=blue:s=32x32:r=10:d=1.8 -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1.8 -c:v libx264 -pix_fmt yuv420p -g 6 -threads 1 -c:a aac -b:a 32k -f dash -seg_duration 0.6 -use_template 1 -use_timeline 1 -y tests/fixtures/dash/media/manifest.mpd
```

The generated manifest has separate H.264 and AAC representations, initialization
resources and three video/four audio fragments. The sibling manifests reuse
those exact resources:

- `list.mpd` spells out initialization and media URLs in SegmentList.
- `number.mpd` replaces video addressing with an inherited duration template:
  timescale 10240, duration 6144, start number 1. Audio retains its explicit list.
- `ranges.mpd` uses explicit resource BaseURLs and inclusive byte ranges.
  `video-combined.mp4` concatenates `init-stream0.m4s` and the three ordered
  `chunk-stream0-*.m4s` resources; `audio-combined.mp4` concatenates the equivalent
  initialization and four audio resources. Each XML range starts after the prior
  resource and spans its exact file length. Recompute ranges when regenerating.

Required tests verify exact assembled bytes for all four addressing forms.
`YT_DLP_RS_TEST_FFPROBE` additionally verifies H.264/AAC single-track output;
setting both it and `YT_DLP_RS_TEST_FFMPEG` exercises merging and verifies both
tracks in MP4. Neither tool is required by the default suite or native extraction.
