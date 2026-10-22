#!/usr/bin/env bash
# Execute the pinned Rust dependency's saved-player vectors with actual Node.
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
native_node="$(command -v node)"
native_manifest="$(cargo metadata --locked --format-version 1 | "$native_node" -e 'let input="";process.stdin.on("data",c=>input+=c);process.stdin.on("end",()=>{const pkg=JSON.parse(input).packages.find(p=>p.name==="yt-dlp-rs");if(!pkg)process.exit(2);console.log(pkg.manifest_path);});')"
YT_DLP_RS_TEST_NODE="$native_node" CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}" cargo test --manifest-path "$native_manifest" --lib --locked real_node_
