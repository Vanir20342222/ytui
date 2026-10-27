# Generated media fixtures

These tiny original test assets were generated offline using FFmpeg 9.0.2.
No third-party media was used. Regenerate from the repository root:

```sh
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i color=c=blue:s=32x32:r=10:d=0.3 -an -c:v libx264 -pix_fmt yuv420p -threads 1 -metadata creation_time=2000-01-01T00:00:00Z -y tests/fixtures/merge/video.mp4
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.3 -vn -c:a aac -b:a 32k -threads 1 -metadata creation_time=2000-01-01T00:00:00Z -y tests/fixtures/merge/audio.m4a
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i color=c=blue:s=32x32:r=10:d=0.3 -an -c:v libvpx-vp9 -threads 1 -y tests/fixtures/merge/video.webm
ffmpeg -nostdin -hide_banner -loglevel error -f lavfi -i sine=frequency=440:sample_rate=48000:duration=0.3 -vn -c:a libopus -b:a 32k -threads 1 -y tests/fixtures/merge/audio.webm
```

The real-tool test serves these files over localhost, merges them by copying
streams, and inspects the resulting H.264/AAC, VP9/Opus and mixed H.264/Opus audio/video tracks using FFprobe.
The required suite uses native process fixtures and needs no FFmpeg installation.
