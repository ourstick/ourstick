#!/bin/bash
# Signs release files with the release key, kept off every build machine
# (release-keys/README.md): each <file> gets <file>.minisig beside it, which
# is then checked against the public key in the repository, so a signature
# made with any other key never goes out.
#   OURSTICK_RELEASE_KEY=<the secret key file> tools/release/sign.sh <file>...
# minisign asks for the key's passphrase, once per file, on the terminal.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
public="$here/../../release-keys/ourstick-release.pub"
secret=${OURSTICK_RELEASE_KEY:?set it to the release key file (the secret one)}
[ $# -ge 1 ] || { echo "usage: $0 <file>..." >&2; exit 2; }
command -v minisign >/dev/null || { echo "minisign is not installed" >&2; exit 1; }
[ -f "$secret" ] || { echo "no such key file: $secret" >&2; exit 1; }

for file in "$@"; do
  [ -f "$file" ] || { echo "no such file: $file" >&2; exit 1; }
  # The trusted comment is signed too: it says what the file is.
  minisign -S -s "$secret" -m "$file" -t "OurStick $(basename "$file")"
  if ! minisign -V -q -p "$public" -m "$file"; then
    rm -f "$file.minisig"
    echo "not the key of release-keys/ourstick-release.pub; the signature is removed" >&2
    exit 1
  fi
  echo "Signed: $file.minisig"
done
