#!/usr/bin/env bash
# Explicit live sweep of the same public prerecorded video, jNQXAC9IVRw.
# No browser import, consent action, VPN change or tool installation.
set -uo pipefail
trap 'exit 130' INT
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
video_failures=()
video_checks=0
run_check() {
    local label="$1"
    shift
    video_checks=$((video_checks + 1))
    printf '\nChecking %s\n' "$label"
    if "$@"; then printf 'PASS %s\n' "$label"
    else video_failures+=("$label"); printf 'FAIL %s\n' "$label" >&2; fi
}
run_check build-harness cargo build -p ytui --locked --example playback_sweep
video_binary="${CARGO_TARGET_DIR:-target}/debug/examples/playback_sweep"
run_check offline-faults cargo test --workspace --locked
run_check actual-local-tools cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2
run_check lua-eof-menu lua crates/ytui-services/tests/player_menu.lua
run_check native-player "$video_binary"
run_check auto-player "$video_binary" --backend auto --cases completion,controls-preview,audio,truncated-eof
run_check external-player "$video_binary" --backend yt-dlp --cases completion,controls-preview,audio
# This deliberate hung-player check takes about 90 seconds, then must fail cleanly.
run_check stalled-startup "$video_binary" --backend yt-dlp --cases no-ipc
run_check native-cli env YTUI_LIVE_BACKEND=native cargo test -p ytui --locked --test live_youtube -- --ignored --test-threads=2 --nocapture --skip public_search --skip playlist
run_check external-cli env YTUI_LIVE_BACKEND=yt-dlp cargo test -p ytui --locked --test live_youtube -- --ignored --test-threads=2 --nocapture --skip public_search --skip playlist
run_check formatting cargo fmt --all -- --check
run_check clippy cargo clippy --workspace --all-targets --locked -- -D warnings
printf '\nCompleted %d groups; %d failed.\n' "$video_checks" "${#video_failures[@]}"
if ((${#video_failures[@]})); then printf 'Failed: %s\n' "${video_failures[@]}"; exit 1; fi
