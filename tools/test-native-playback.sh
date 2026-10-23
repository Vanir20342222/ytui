#!/usr/bin/env bash
# Explicit public native-only playback checks; no yt-dlp or browser session.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
native_urls="${1:-tools/native-playback-urls.txt}"
native_binary="${YTUI_TEST_BINARY:-target/debug/ytui}"
[[ -f "$native_urls" ]] || { echo 'Provide a text file containing one public video URL per line.' >&2; exit 2; }
[[ -x "$native_binary" ]] || { echo 'Build first: cargo build -p ytui --locked' >&2; exit 2; }
native_mpv="$(command -v "${YTUI_TEST_MPV:-mpv}")"
native_root="$(mktemp -d "${TMPDIR:-/tmp}/ytui-native.XXXXXX")"
trap 'rm -rf -- "$native_root"' EXIT
chmod 700 "$native_root"
printf '#!/usr/bin/env bash\nexec %q --vo=null --ao=null "$@"\n' "$native_mpv" > "$native_root/mpv"
chmod 700 "$native_root/mpv"
cat > "$native_root/config.toml" <<EOF
theme = 'ytui-dark'
auth_store = 'file'
notifications = false
download_backend = 'native'
playback_backend = 'native'
playback_sponsorblock = 'off'
yt_dlp = '/native-playback-must-never-use-yt-dlp'
mpv = 'mpv'
EOF
export PATH="$native_root:$PATH"
export XDG_CONFIG_HOME="$native_root/config"
export XDG_STATE_HOME="$native_root/state"
export XDG_CACHE_HOME="$native_root/cache"
export XDG_DATA_HOME="$native_root/data"
native_total=0
native_passed=0
while IFS= read -r native_url || [[ -n "$native_url" ]]; do
  native_url="${native_url%$'\r'}"
  [[ -z "$native_url" || "$native_url" == \#* ]] && continue
  native_total=$((native_total + 1))
  echo "Native case $native_total"
  native_mode=()
  [[ "$native_url" == https://music.youtube.com/* ]] && native_mode+=(--audio)
  if "$native_binary" --config "$native_root/config.toml" --state-dir "$native_root/state" playback-check "$native_url" --backend native --seconds "${YTUI_TEST_PLAY_SECONDS:-3}" "${native_mode[@]}"; then
    native_passed=$((native_passed + 1))
  fi
done < <(awk '!seen[$0]++' "$native_urls")
[[ "$native_total" -gt 0 ]] || { echo 'No public video URLs were supplied.' >&2; exit 2; }
awk -v passed="$native_passed" -v total="$native_total" 'BEGIN { printf "Native startup playback: %d/%d passed (%.2f%% observed).\n", passed,total,100*passed/total }'
if [[ "$native_total" -lt 299 ]]; then
  echo 'This is a smoke result, not evidence of a 99% population success rate.'
else
  echo 'A reliability claim also requires independent, representative video/account/network cases.'
fi
(( native_passed * 100 >= native_total * 99 ))
