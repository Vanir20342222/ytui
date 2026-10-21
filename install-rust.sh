#!/usr/bin/env bash
# Installs alongside the Python release. Does not replace its launcher or state.
set -euo pipefail
if ! command -v cargo >/dev/null 2>&1; then
  echo 'Install current stable Rust first: https://rustup.rs' >&2
  exit 1
fi
ytui_source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ytui_install_dir="${YTUI_INSTALL_DIR:-$HOME/.local/bin}"
cargo build --manifest-path "$ytui_source_dir/Cargo.toml" --release --locked -p ytui
mkdir -p -- "$ytui_install_dir"
install -m 755 -- "$ytui_source_dir/target/release/ytui" "$ytui_install_dir/ytui-rs"
"$ytui_install_dir/ytui-rs" --version
printf 'Installed %s/ytui-rs\nRun ytui-rs doctor, then ytui-rs.\n' "$ytui_install_dir"
