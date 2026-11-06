#!/bin/bash
# Prints one SHA-256 for the built frontend (apps/app/dist, or the folder
# given): the hash of every file's hash, in a fixed order. Run it where the
# frontend was built, and give the result to make-dmg.sh on the Mac as
# OURSTICK_DIST_SHA256, so the app is signed only with the very files that
# were built and checked, not with whatever the copy left there.
#
#   tools/macos/dist-hash.sh [folder]
set -euo pipefail

dir=${1:-$(cd "$(dirname "$0")/../.." && pwd)/apps/app/dist}
cd "$dir"
# A link would put a file from elsewhere inside the app.
if [ -n "$(find . -type l -print -quit)" ]; then
  echo "$dir holds a link: refused." >&2
  exit 1
fi
if command -v sha256sum >/dev/null; then sum=(sha256sum); else sum=(shasum -a 256); fi
find . -type f -print0 | LC_ALL=C sort -z | xargs -0 "${sum[@]}" | "${sum[@]}" | cut -d' ' -f1
