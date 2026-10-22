#!/usr/bin/env bash
# Explicit comprehensive local acceptance, including public network/media access.
# Tests isolate state; no browser import, consent action, VPN change or tool install.
set -uo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
network_checks=0
network_failures=()
run_check() {
    local label="$1"
    shift
    network_checks=$((network_checks + 1))
    printf '\nChecking %s\n' "$label"
    if "$@"; then
        printf 'PASS %s\n' "$label"
    else
        network_failures+=("$label")
        printf 'FAIL %s\n' "$label" >&2
    fi
}
run_check formatting cargo fmt --all -- --check
run_check clippy cargo clippy --workspace --all-targets --locked -- -D warnings
run_check offline cargo test --workspace --locked
run_check native-solver tools/test-native-solver.sh
run_check player-menu lua crates/ytui-services/tests/player_menu.lua
run_check actual-local-tools cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2
run_check public-radio-youtube-cache cargo test -p ytui-services --locked --test live_network -- --ignored --test-threads=2 --nocapture
run_check youtube-auto env YTUI_LIVE_BACKEND=auto cargo test -p ytui --locked --test live_youtube -- --ignored --skip live_native_ --test-threads=2 --nocapture
run_check youtube-external env YTUI_LIVE_BACKEND=yt-dlp cargo test -p ytui --locked --test live_youtube -- --ignored --skip live_native_ --test-threads=2 --nocapture
run_check native-download cargo test -p ytui --locked --test live_youtube live_native_ -- --ignored --nocapture
run_check native-playback env YTUI_TEST_PLAY_SECONDS=10 tools/test-native-playback.sh
run_check release-build cargo build --release --locked -p ytui
printf '\nCompleted %d check groups; %d failed.\n' "$network_checks" "${#network_failures[@]}"
if ((${#network_failures[@]})); then
    printf 'Failed: %s\n' "${network_failures[@]}"
    exit 1
fi
