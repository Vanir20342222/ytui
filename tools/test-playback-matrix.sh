#!/usr/bin/env bash
# Opt-in public corpus. No browser import, consent action, VPN change or install.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
matrix_urls="${1:-tools/native-playback-urls.txt}"
matrix_binary="${YTUI_TEST_BINARY:-target/debug/ytui}"
matrix_output="${YTUI_MATRIX_OUTPUT:-playback-matrix-$(date -u +%Y%m%dT%H%M%SZ)}"
[[ -f "$matrix_urls" && -x "$matrix_binary" ]] || { echo 'Build ytui and provide a public URL corpus.' >&2; exit 2; }
# A new output directory is mandatory; never replace an earlier acceptance run.
mkdir -- "$matrix_output"
matrix_output="$(cd -- "$matrix_output" && pwd)"
matrix_mpv="$(command -v "${YTUI_TEST_MPV:-mpv}")"
matrix_root="$(mktemp -d "${TMPDIR:-/tmp}/ytui-matrix.XXXXXX")"
trap 'rm -rf -- "$matrix_root"' EXIT
printf '#!/usr/bin/env bash\nexec %q --vo=null --ao=null "$@"\n' "$matrix_mpv" > "$matrix_root/mpv"
chmod 700 "$matrix_root/mpv"
cat > "$matrix_root/config.toml" <<EOF
theme = 'ytui-dark'
auth_store = 'file'
notifications = false
playback_backend = 'native'
playback_sponsorblock = 'off'
yt_dlp = '/native-matrix-must-not-use-external-extraction'
mpv = 'mpv'
EOF
export PATH="$matrix_root:$PATH"
export XDG_CONFIG_HOME="$matrix_root/config"
export XDG_STATE_HOME="$matrix_root/state"
export XDG_CACHE_HOME="$matrix_root/cache"
export XDG_DATA_HOME="$matrix_root/data"
"$matrix_binary" --version > "$matrix_output/build.txt"
printf 'case\tmode\tmaximum_height\trequested_seconds\texit\twall_seconds\n' > "$matrix_output/results.tsv"
matrix_case=0
matrix_failed=0
while IFS= read -r matrix_url || [[ -n "$matrix_url" ]]; do
  matrix_url="${matrix_url%$'\r'}"
  [[ -z "$matrix_url" || "$matrix_url" == \#* ]] && continue
  for matrix_quality in ${YTUI_MATRIX_QUALITIES:-1080 1440 2160}; do
    matrix_case=$((matrix_case + 1))
    matrix_mode=video
    matrix_audio=()
    if [[ "$matrix_url" == https://music.youtube.com/* ]]; then matrix_mode=audio; matrix_audio+=(--audio); fi
    matrix_started="$SECONDS"
    matrix_exit=0
    "$matrix_binary" --config "$matrix_root/config.toml" --state-dir "$matrix_root/state" playback-check "$matrix_url" --backend native --quality "$matrix_quality" --seconds "${YTUI_TEST_PLAY_SECONDS:-30}" "${matrix_audio[@]}" > "$matrix_output/case-$matrix_case.json" 2> "$matrix_output/case-$matrix_case.log" || matrix_exit=$?
    printf '%d\t%s\t%s\t%s\t%d\t%d\n' "$matrix_case" "$matrix_mode" "$matrix_quality" "${YTUI_TEST_PLAY_SECONDS:-30}" "$matrix_exit" "$((SECONDS - matrix_started))" | tee -a "$matrix_output/results.tsv"
    if ((matrix_exit)); then matrix_failed=$((matrix_failed + 1)); fi
    # One audio case per URL; a video height cap does not create a new audio case.
    [[ "$matrix_mode" == audio ]] && break
  done
done < "$matrix_urls"
printf '%d cases, %d failed. Logs: %s\n' "$matrix_case" "$matrix_failed" "$matrix_output"
echo 'Requested height is a cap, not proof the source offers it. Inspect decoded_height/codecs. This campaign is not a 99% reliability estimate.'
((matrix_case > 0 && matrix_failed == 0))
