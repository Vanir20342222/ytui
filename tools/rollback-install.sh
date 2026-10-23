#!/usr/bin/env bash
# Explicit executable-only rollback. Does not touch configuration or state.
set -euo pipefail
rollback_bin="${YTUI_INSTALL_DIR:-$HOME/.local/bin}"
[[ -f "$rollback_bin/ytui.previous" && ! -L "$rollback_bin/ytui.previous" ]] || { echo 'No previous executable snapshot exists.' >&2; exit 1; }
for rollback_name in ytui ytui-rs; do
  [[ ! -d "$rollback_bin/$rollback_name" ]] || { echo 'A launcher path is a directory.' >&2; exit 1; }
done
rollback_stage="$(mktemp -d "$rollback_bin/.ytui-rollback.XXXXXX")"
trap 'rm -rf -- "$rollback_stage"' EXIT
install -m755 -- "$rollback_bin/ytui.previous" "$rollback_stage/ytui"
"$rollback_stage/ytui" --version
ln -- "$rollback_stage/ytui" "$rollback_stage/ytui-rs"
# Keep the newer executable for an explicit roll-forward.
if [[ -f "$rollback_bin/ytui" && ! -L "$rollback_bin/ytui" ]]; then install -m755 -- "$rollback_bin/ytui" "$rollback_stage/next"; fi
mv -fT -- "$rollback_stage/ytui" "$rollback_bin/ytui"
mv -fT -- "$rollback_stage/ytui-rs" "$rollback_bin/ytui-rs"
if [[ -f "$rollback_stage/next" ]]; then mv -fT -- "$rollback_stage/next" "$rollback_bin/ytui.previous"; fi
printf 'Executable rollback complete. Restart ytui. Existing state was not migrated or replaced.\n'
