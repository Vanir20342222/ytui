#!/usr/bin/env bash
# Offline regression of the actual CI installer, with no root/package changes.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
installer="${1:-tools/ci-install-media.sh}"
test_root="$(mktemp -d)"
trap 'rm -rf -- "$test_root"' EXIT
mkdir "$test_root/bin"
cat > "$test_root/bin/sudo" <<'EOF'
#!/usr/bin/env bash
[[ "$1" == apt-get ]] || exit 99
exec "$@"
EOF
cat > "$test_root/bin/lsb_release" <<'EOF'
#!/usr/bin/env bash
case "$1" in -is) echo Ubuntu;; -cs) echo noble;; *) exit 99;; esac
EOF
cat > "$test_root/bin/uname" <<'EOF'
#!/usr/bin/env bash
echo x86_64
EOF
cat > "$test_root/bin/rg" <<'EOF'
#!/usr/bin/env bash
# GitHub's minimal Ubuntu image does not provide ripgrep.
exit 127
EOF
cat > "$test_root/bin/timeout" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CI_TEST_LOG.timeout"
[[ "$1" == --kill-after=10s && ( "$2" == 60s || "$2" == 180s ) ]] || exit 99
shift 2
exec /usr/bin/timeout --kill-after=0.1s 0.5s "$@"
EOF
cat > "$test_root/bin/apt-get" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CI_TEST_LOG"
fallback=false
operation=''
for arg; do
    case "$arg" in update|install) operation="$arg";; esac
    if [[ "$arg" == Dir::Etc::sourcelist=* ]]; then
        fallback=true
        source_file="${arg#*=}"
        [[ -f "$source_file" ]] || exit 99
        cp "$source_file" "$CI_TEST_LOG.sources"
    fi
done
case "$CI_TEST_CASE" in
    primary_success) exit 0;;
    primary_install_stall) "$fallback" || [[ "$operation" != install ]] || sleep 8;;
    primary_update_stall) "$fallback" || [[ "$operation" != update ]] || sleep 8;;
    primary_update_failure) "$fallback" || exit 100;;
    primary_install_failure) "$fallback" || [[ "$operation" != install ]] || exit 100;;
    all_failure) exit 100;;
    *) exit 99;;
esac
EOF
chmod +x "$test_root/bin/"*
export PATH="$test_root/bin:$PATH"
export GITHUB_ACTIONS=true RUNNER_OS=Linux
for test_case in primary_install_stall primary_update_stall primary_success primary_update_failure primary_install_failure all_failure; do
    export CI_TEST_CASE="$test_case" CI_TEST_LOG="$test_root/$test_case"
    status=0
    /usr/bin/timeout --kill-after=1s 5s bash "$installer" mpv ffmpeg > "$test_root/output" 2>&1 || status=$?
    if [[ "$test_case" == all_failure ]]; then
        [[ "$status" == 1 ]] || { cat "$test_root/output"; echo 'FAIL: failed mirrors must fail setup'; exit 1; }
    else
        [[ "$status" == 0 ]] || { cat "$test_root/output"; echo "FAIL: $test_case was not recovered (exit $status)"; exit 1; }
    fi
    [[ -s "$CI_TEST_LOG.timeout" ]] || { echo 'FAIL: apt operations lack deadlines'; exit 1; }
    if [[ "$test_case" != all_failure ]]; then
        grep -Eq ' install -y --no-install-recommends mpv ffmpeg$' "$CI_TEST_LOG"
    fi
    if [[ "$test_case" == primary_success ]]; then
        [[ ! -e "$CI_TEST_LOG.sources" ]] || { echo 'FAIL: unnecessary fallback'; exit 1; }
    else
        [[ -s "$CI_TEST_LOG.sources" ]]
        grep -Eq '^deb https://archive.ubuntu.com/ubuntu noble ' "$CI_TEST_LOG.sources"
        grep -Eq '^deb https://security.ubuntu.com/ubuntu noble-security ' "$CI_TEST_LOG.sources"
        [[ "$(grep -Ec ' update$' "$CI_TEST_LOG")" == 2 ]]
        [[ "$(grep -Ec 'Dir::Etc::sourceparts=-' "$CI_TEST_LOG")" -ge 1 ]]
    fi
    echo "PASS: $test_case"
done
# A misleading environment flag alone must not invoke apt on an ordinary host.
export CI_TEST_LOG="$test_root/refused"
if GITHUB_ACTIONS=false bash "$installer" mpv > "$test_root/output" 2>&1; then
    echo 'FAIL: installer accepted a non-CI environment'; exit 1
fi
[[ ! -e "$CI_TEST_LOG" ]]
if bash "$installer" --allow-unauthenticated > "$test_root/output" 2>&1; then
    echo 'FAIL: installer accepted an unsupported argument'; exit 1
fi
[[ ! -e "$CI_TEST_LOG" ]]
echo 'PASS: host and package argument guards'
