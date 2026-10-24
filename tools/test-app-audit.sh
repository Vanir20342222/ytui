#!/usr/bin/env bash
# Explicit audit regressions. They intentionally fail on e2b01ea; no live services.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
audit_flags=()
[[ $# -le 1 ]] || { echo 'Usage: tools/test-app-audit.sh [--real]' >&2; exit 2; }
case "${1:-}" in
  '') ;;
  --real) audit_flags+=(--include-ignored) ;;
  *) echo 'Usage: tools/test-app-audit.sh [--real] (adds actual local FFmpeg/ffprobe check)' >&2; exit 2 ;;
esac
audit_target='crates/ytui/tests/app_review_repros.rs'
if ! (set -o noclobber; cat -- docs/audit/app_review_repros.rs > "$audit_target"); then
    echo "Refusing to overwrite $audit_target" >&2
    exit 2
fi
trap 'rm -f -- "$audit_target"' EXIT
echo 'Audit baseline: desired-behavior assertions fail until the reported bugs are fixed.'
cargo test -p ytui --test app_review_repros --locked -- "${audit_flags[@]}" --nocapture
