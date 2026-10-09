#!/bin/bash
# Makes THIRD-PARTY-LICENSES.html at the repository's root, shipped in every
# package: `cargo install cargo-about --locked --features cli`, then run it.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
cargo about generate --workspace --config tools/licenses/about.toml \
  --output-file THIRD-PARTY-LICENSES.html tools/licenses/about.hbs
echo "Made $root/THIRD-PARTY-LICENSES.html"
