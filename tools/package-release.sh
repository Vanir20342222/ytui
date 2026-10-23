#!/usr/bin/env bash
# Deterministic archive layout/timestamps for a reviewed build; no publication.
set -euo pipefail
umask 022
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
release_output="${1:-dist}"
release_revision="$(git rev-parse HEAD)"
export YTUI_BUILD_REVISION="$release_revision"
release_epoch="${SOURCE_DATE_EPOCH:-$(git show -s --format=%ct HEAD)}"
[[ "$release_epoch" =~ ^[0-9]+$ ]] || { echo 'SOURCE_DATE_EPOCH must be numeric.' >&2; exit 2; }
[[ -z "$(git status --porcelain)" ]] || { echo 'Commit reviewed source changes before packaging.' >&2; exit 2; }
cargo build --release --locked -p ytui
release_version="$("${CARGO_TARGET_DIR:-target}/release/ytui" --version | awk '{print $2}')"
release_name="ytui-$release_version-$(uname -s)-$(uname -m)"
mkdir -p -- "$release_output"
release_output="$(cd -- "$release_output" && pwd)"
[[ ! -e "$release_output/$release_name.tar.gz" ]] || { echo 'Archive already exists; choose a new output directory.' >&2; exit 2; }
release_stage="$(mktemp -d "${TMPDIR:-/tmp}/ytui-release.XXXXXX")"
trap 'rm -rf -- "$release_stage"' EXIT
mkdir -- "$release_stage/$release_name"
install -m755 "${CARGO_TARGET_DIR:-target}/release/ytui" "$release_stage/$release_name/ytui"
ln -s ytui "$release_stage/$release_name/ytui-rs"
cp -- LICENSE Cargo.lock docs/RUST.md packaging/ytui.desktop "$release_stage/$release_name/"
cargo metadata --locked --format-version 1 > "$release_stage/dependencies.json"
cargo run --release --locked -p ytui --example license_bundle -- "$release_stage/dependencies.json" "$release_stage/$release_name/licenses"
{
  printf 'Revision: %s\nVersion: %s\nSource date epoch: %s\n' "$release_revision" "$release_version" "$release_epoch"
  rustc --version
  cargo --version
  printf 'Archive layout/timestamps are normalized. Binary reproducibility requires the same compiler, target, dependencies and build environment.\n'
} > "$release_stage/$release_name/BUILD.txt"
tar --sort=name --mtime="@$release_epoch" --owner=0 --group=0 --numeric-owner -C "$release_stage" -cf - "$release_name" | gzip -n > "$release_output/$release_name.tar.gz"
(cd -- "$release_output" && sha256sum "$release_name.tar.gz" > "$release_name.sha256")
printf 'Prepared %s\nNo tag, release or package was published.\n' "$release_output/$release_name.tar.gz"
