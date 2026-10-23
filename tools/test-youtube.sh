#!/usr/bin/env bash
# Explicit live checks: isolated state, public URLs, actual media tools, no browser import.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
export YTUI_LIVE_BACKEND="${1:-auto}"
case "$YTUI_LIVE_BACKEND" in
  auto|native|yt-dlp) ;;
  *) echo 'Usage: tools/test-youtube.sh [auto|native|yt-dlp]' >&2; exit 2 ;;
esac
for tool in cargo ffmpeg ffprobe "${YTUI_TEST_MPV:-mpv}" "${YTUI_TEST_YT_DLP:-yt-dlp}"; do
  if ! command -v "$tool" >/dev/null; then
    echo "Required test tool is missing: $tool" >&2
    exit 2
  fi
done
cargo --version
ffmpeg -version | sed -n '1p'
"${YTUI_TEST_MPV:-mpv}" --version | sed -n '1p'
"${YTUI_TEST_YT_DLP:-yt-dlp}" --version
if command -v deno >/dev/null; then deno --version | sed -n '1p'; fi
echo "Testing public YouTube with backend: $YTUI_LIVE_BACKEND"
echo 'Temporary media and isolated test state are deleted after each check.'
# Strict native is a separate diagnostic: its known challenge limitations must not
# turn a successful auto/external suite into a false failure (or a false pass).
exec cargo test -p ytui --locked --test live_youtube -- --ignored --test-threads=1 --nocapture --skip live_native_youtube_test_video_download
