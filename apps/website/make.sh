#!/bin/bash
# Puts together the site for Cloudflare Pages in apps/website/dist: the pages,
# the third-party licences, and the three installers, given by path:
#   apps/website/make.sh <OurStick.dmg> <OurStick-setup.exe> <OurStick.deb>
# The pages get each installer's size. Pages takes files of up to 25 MiB.
# Deploying: apps/website/README.md.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 3 ] || { echo "usage: $0 <dmg> <setup.exe> <deb>" >&2; exit 2; }
case "$1" in *.dmg) ;; *) echo "not a .dmg: $1" >&2; exit 2 ;; esac
case "$2" in *.exe) ;; *) echo "not a .exe: $2" >&2; exit 2 ;; esac
case "$3" in *.deb) ;; *) echo "not a .deb: $3" >&2; exit 2 ;; esac

size_of() { stat -c %s "$1" 2>/dev/null || stat -f %z "$1"; }
for file in "$@"; do
  [ -f "$file" ] || { echo "no such file: $file" >&2; exit 1; }
  [ "$(size_of "$file")" -le 26214400 ] || { echo "over 25 MiB, Pages refuses it: $file" >&2; exit 1; }
done
megabytes() { echo "$(( ($(size_of "$1") + 524288) / 1048576 )) MB"; }

rm -rf "$here/dist"
cp -R "$here/site" "$here/dist"
cp "$here/../../THIRD-PARTY-LICENSES.html" "$here/dist/"
for page in "$here/dist/index.html" "$here/dist/s/index.html"; do
  sed -e "s/SIZE_MACOS/$(megabytes "$1")/" -e "s/SIZE_WINDOWS/$(megabytes "$2")/" \
    -e "s/SIZE_LINUX/$(megabytes "$3")/" "$page" > "$page.new"
  mv "$page.new" "$page"
done
if grep -rq "SIZE_" "$here/dist" --include=*.html; then echo "a size left out" >&2; exit 1; fi

mkdir "$here/dist/downloads"
cp "$1" "$here/dist/downloads/OurStick-macOS.dmg"
cp "$2" "$here/dist/downloads/OurStick-Windows-setup.exe"
cp "$3" "$here/dist/downloads/OurStick-Linux-amd64.deb"
cd "$here/dist/downloads"
names=(OurStick-macOS.dmg OurStick-Windows-setup.exe OurStick-Linux-amd64.deb)
if command -v sha256sum >/dev/null; then sums=$(sha256sum "${names[@]}"); else sums=$(shasum -a 256 "${names[@]}"); fi
printf '%s\n' "$sums" > SHA256SUMS.txt
echo "Made $here/dist"
