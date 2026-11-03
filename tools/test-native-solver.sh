#!/usr/bin/env bash
# Execute the pinned Rust dependency's saved-player vectors with actual Node.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
# Like the application, skip an unconfigured shim hiding a supported later Node.
native_node=''
while IFS= read -r native_candidate; do
  native_version="$(timeout 4s "$native_candidate" --version 2>/dev/null)" || continue
  if [[ "$native_version" =~ ^v([0-9]+)\. ]] && (( BASH_REMATCH[1] >= 22 )); then
    # Resolve version-manager shims while still in this repository's context.
    # Cargo runs dependency tests from a different working directory.
    native_resolved="$(timeout 4s "$native_candidate" -p 'process.execPath' 2>/dev/null)" || continue
    [[ "$native_resolved" = /* && -x "$native_resolved" ]] || continue
    native_node="$native_resolved"
    break
  fi
done < <(type -a -P node)
[[ -n "$native_node" ]] || { echo 'Install Node.js 22+ to run the native solver vectors.' >&2; exit 2; }
native_manifest="$(cargo metadata --locked --format-version 1 | "$native_node" -e 'let input="";process.stdin.on("data",c=>input+=c);process.stdin.on("end",()=>{const pkg=JSON.parse(input).packages.find(p=>p.name==="yt-dlp-rs");if(!pkg)process.exit(2);console.log(pkg.manifest_path);});')"
YT_DLP_RS_TEST_NODE="$native_node" CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}" cargo test --manifest-path "$native_manifest" --lib --locked real_node_
