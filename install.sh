#!/usr/bin/env bash
# Build/install the Rust app. Existing settings, media and legacy data stay intact.
set -euo pipefail
if ! command -v cargo >/dev/null 2>&1; then
    printf 'Install current stable Rust first: https://rustup.rs\n' >&2
    exit 1
fi
ytui_source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ytui_source_tmp=''
ytui_publish_tmp=''
cleanup() {
    if [[ -n "$ytui_source_tmp" ]]; then rm -rf -- "$ytui_source_tmp"; fi
    if [[ -n "$ytui_publish_tmp" ]]; then rm -rf -- "$ytui_publish_tmp"; fi
}
trap cleanup EXIT
if [[ ! -f "$ytui_source_dir/Cargo.toml" || ! -d "$ytui_source_dir/apps/desktop" ]]; then
    command -v git >/dev/null 2>&1 || { printf 'Install git first.\n' >&2; exit 1; }
    ytui_source_tmp="$(mktemp -d "${TMPDIR:-/tmp}/ytui-source.XXXXXX")"
    git clone --depth 1 --branch "${YTUI_BRANCH:-main}" \
        https://github.com/Vanir20342222/ytui.git "$ytui_source_tmp/repo"
    ytui_source_dir="$ytui_source_tmp/repo"
fi
ytui_install_dir="${YTUI_INSTALL_DIR:-$HOME/.local/bin}"
ytui_build_dir="${CARGO_TARGET_DIR:-$ytui_source_dir/target}"
cargo build --manifest-path "$ytui_source_dir/Cargo.toml" --target-dir "$ytui_build_dir" --release --locked -p ytui
mkdir -p -- "$ytui_install_dir"
ytui_publish_tmp="$(mktemp -d "$ytui_install_dir/.ytui-install.XXXXXX")"
install -m 755 -- "$ytui_build_dir/release/ytui" "$ytui_publish_tmp/ytui"
"$ytui_publish_tmp/ytui" --version
for ytui_launcher in ytui ytui-rs ytui.previous; do
    [[ ! -d "$ytui_install_dir/$ytui_launcher" ]] || { printf 'Installation target is a directory: %s\n' "$ytui_install_dir/$ytui_launcher" >&2; exit 1; }
done
if [[ -f "$ytui_install_dir/ytui" && ! -L "$ytui_install_dir/ytui" ]] && ! cmp -s -- "$ytui_install_dir/ytui" "$ytui_publish_tmp/ytui"; then
    install -m755 -- "$ytui_install_dir/ytui" "$ytui_publish_tmp/ytui.previous"
    mv -fT -- "$ytui_publish_tmp/ytui.previous" "$ytui_install_dir/ytui.previous"
fi
ln -- "$ytui_publish_tmp/ytui" "$ytui_publish_tmp/ytui-rs"
# Replace an old launcher/symlink itself, never write through it into a virtualenv.
mv -fT -- "$ytui_publish_tmp/ytui" "$ytui_install_dir/ytui"
mv -fT -- "$ytui_publish_tmp/ytui-rs" "$ytui_install_dir/ytui-rs"
printf 'Installed Rust launchers: %s/ytui and %s/ytui-rs\nRun ytui doctor, then ytui.\n' "$ytui_install_dir" "$ytui_install_dir"
printf 'Restart running ytui processes to use this build. Previous changed executable: ytui.previous (when available).\n'
