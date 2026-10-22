#!/usr/bin/env bash
# GitHub-hosted Ubuntu 24.04 x64 only. Never install tools on a developer's PC.
set -euo pipefail
if [[ "${GITHUB_ACTIONS:-}" != true || "${RUNNER_OS:-}" != Linux ]] ||
    [[ "$(lsb_release -is)" != Ubuntu || "$(lsb_release -cs)" != noble || "$(uname -m)" != x86_64 ]]; then
    echo 'This installer is only for Ubuntu 24.04 x64 GitHub Actions runners.' >&2
    exit 2
fi
(( $# > 0 )) || { echo 'Specify mpv, ffmpeg and/or lua5.4.' >&2; exit 2; }
for package; do
    case "$package" in mpv|ffmpeg|lua5.4) ;; *) echo 'Unsupported CI media package.' >&2; exit 2;; esac
done
apt_options=(
    -o Acquire::Retries=1
    -o Acquire::http::Timeout=15
    -o Acquire::https::Timeout=15
    -o APT::Update::Error-Mode=any
)
install_media() {
    # A per-request timeout alone can still stall across a hundred packages.
    # Bound each whole operation, then allow one independent mirror attempt.
    timeout --kill-after=10s 60s sudo apt-get "${apt_options[@]}" update &&
        timeout --kill-after=10s 180s sudo apt-get "${apt_options[@]}" install -y --no-install-recommends "$@"
}
if install_media "$@"; then
    exit 0
fi
echo 'Runner package mirror failed or timed out; retrying official Ubuntu HTTPS mirrors.' >&2
fallback_sources="$(mktemp "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/ytui-ubuntu.XXXXXX.list")"
trap 'rm -f -- "$fallback_sources"' EXIT
cat > "$fallback_sources" <<'EOF'
deb https://archive.ubuntu.com/ubuntu noble main restricted universe multiverse
deb https://archive.ubuntu.com/ubuntu noble-updates main restricted universe multiverse
deb https://security.ubuntu.com/ubuntu noble-security main restricted universe multiverse
EOF
# Override sources for these commands only; preserve the runner's source files.
# Apt drops download privileges to _apt, which must be able to read this file.
chmod 644 "$fallback_sources"
apt_options+=(
    -o "Dir::Etc::sourcelist=$fallback_sources"
    -o Dir::Etc::sourceparts=-
)
if install_media "$@"; then
    exit 0
fi
echo 'CI media setup failed on both mirrors. No playback test result is available.' >&2
exit 1
