#!/usr/bin/env bash
# Audit regressions are now part of ordinary Cargo discovery.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
audit_flags=()
[[ $# -le 1 ]] || { echo 'Usage: tools/test-app-audit.sh [--real]' >&2; exit 2; }
case "${1:-}" in
  '') ;;
  --real) audit_flags+=(--include-ignored) ;;
  *) echo 'Usage: tools/test-app-audit.sh [--real]' >&2; exit 2 ;;
esac
exec cargo test -p ytui --test audit_regressions --locked -- "${audit_flags[@]}" --nocapture
