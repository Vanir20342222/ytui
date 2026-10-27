#!/usr/bin/env bash
# Generated, public-domain synthetic media; no YouTube/account access.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
mkdir -p "$root/target/android-test-assets"
ffmpeg -nostdin -hide_banner -loglevel error -y -f lavfi -i 'testsrc2=size=320x180:rate=15' -f lavfi -i 'sine=frequency=440:sample_rate=44100' -t 8 -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -movflags +faststart "$root/target/android-test-assets/fixture.mp4"
ffmpeg -nostdin -hide_banner -loglevel error -y -i "$root/target/android-test-assets/fixture.mp4" -map 0:v -c copy -movflags +faststart "$root/target/android-test-assets/fixture-video.mp4"
ffmpeg -nostdin -hide_banner -loglevel error -y -i "$root/target/android-test-assets/fixture.mp4" -map 0:a -c copy -movflags +faststart "$root/target/android-test-assets/fixture-audio.m4a"
