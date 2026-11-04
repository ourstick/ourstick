#!/bin/bash
# Checks that each crate vendored in third_party/ is the one published on
# crates.io with only the changes its OURSTICK.patch shows: a pull request
# that changes a vendored file elsewhere fails here. With --write, writes
# the patches instead (after a deliberate change, see each OURSTICK.md).
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
crates=("iroh-blobs 0.103.1" "ksni 0.3.6")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
status=0
for crate in "${crates[@]}"; do
  read -r name version <<<"$crate"
  curl -fsSL "https://static.crates.io/crates/$name/$name-$version.crate" -o "$work/$name.crate"
  mkdir -p "$work/$name"
  tar -xzf "$work/$name.crate" -C "$work/$name" --strip-components=1
  vendored="$root/third_party/$name"
  # Every vendored file against the published one (OurStick's notes aside).
  diff=$(cd "$vendored" && find . -type f ! -name OURSTICK.md ! -name OURSTICK.patch | LC_ALL=C sort |
    while read -r file; do
      diff -u --label "a/${file#./}" --label "b/${file#./}" "$work/$name/$file" "$file" || true
    done)
  if [ "${1:-}" = --write ]; then
    printf '%s\n' "$diff" > "$vendored/OURSTICK.patch"
  elif ! diff -q <(printf '%s\n' "$diff") "$vendored/OURSTICK.patch" > /dev/null; then
    echo "third_party/$name differs from $name $version beyond OURSTICK.patch:" >&2
    diff <(printf '%s\n' "$diff") "$vendored/OURSTICK.patch" | head -40 >&2
    status=1
  fi
done
[ "$status" = 0 ] && echo "third_party: as published, with their patches"
exit "$status"
