#!/bin/bash
# Makes apps/website/site/version.json, the signed notice the desktop app
# reads once a day (ADR 0033), after the installers and the release notes
# are up, never before:
#   OURSTICK_RELEASE_KEY=<the release key file> tools/release/version-json.sh <newest> <security>
# <newest> is the version just released; <security> the oldest that is
# safe (the newest itself after a security fix, else the one before's).
# Both for Windows, macOS and Linux. The serial is the committed one's plus
# one. Then commit it, apps/website/make.sh and deploy.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo="$here/../.."
out="$repo/apps/website/site/version.json"
secret=${OURSTICK_RELEASE_KEY:?set it to the release key file (the secret one)}
[ $# -eq 2 ] || { echo "usage: $0 <newest> <security>" >&2; exit 2; }
newest=$1 security=$2
plain='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
for version in "$newest" "$security"; do
  [[ $version =~ $plain ]] || { echo "not X.Y.Z: $version" >&2; exit 2; }
done
[ "$(printf '%s\n%s\n' "$security" "$newest" | sort -V | tail -1)" = "$newest" ] ||
  { echo "security $security is above newest $newest" >&2; exit 2; }
workspace=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$repo/Cargo.toml" | head -1)
[ "$workspace" = "$newest" ] ||
  { echo "newest $newest is not the workspace's version $workspace" >&2; exit 2; }

last=0
if [ -f "$out" ]; then
  last=$(python3 -I -c 'import json,sys; print(json.loads(json.load(open(sys.argv[1]))["notice"])["serial"])' "$out")
fi
serial=$((last + 1))
made=$(date -u +%Y-%m-%d)

work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT
python3 -I - "$serial" "$made" "$newest" "$security" > "$work/notice" <<'PY'
import json, sys
serial, made, newest, security = sys.argv[1:]
each = {"newest": newest, "security": security}
print(json.dumps({"serial": int(serial), "made": made,
                  "windows": each, "macos": each, "linux": each},
                 indent=1), end="")
PY
# The legacy kind (-l): the notice's bytes signed as they are (rule 3).
minisign -S -l -s "$secret" -m "$work/notice" -x "$work/notice.minisig" \
  -t "OurStick version.json $serial"
minisign -V -q -p "$repo/release-keys/ourstick-release.pub" -m "$work/notice" -x "$work/notice.minisig" ||
  { echo "not the key of release-keys/ourstick-release.pub" >&2; exit 1; }
python3 -I - "$work/notice" "$work/notice.minisig" > "$out" <<'PY'
import json, sys
notice = open(sys.argv[1]).read()
signature = open(sys.argv[2]).read()
print(json.dumps({"notice": notice, "signature": signature}, indent=1))
PY
echo "Made $out: serial $serial, newest $newest, security $security"
