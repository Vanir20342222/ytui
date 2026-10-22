#!/usr/bin/env bash
# Compatibility entry point; the default installer now installs Rust.
set -euo pipefail
exec bash "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/install.sh" "$@"
