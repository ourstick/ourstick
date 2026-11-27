#!/bin/bash
# Makes apps/website/site/version.json, the signed notice the desktop app
# reads once a day (ADR 0033), after the installers and the release notes
# are up, never before:
#   OURSTICK_RELEASE_KEY=<the release key file> tools/release/version-json.sh <newest> <security> [<system>...]
# <newest> is the version just released; <security> the oldest that is
# safe (the newest itself after a security fix, else the one before's).
# Both for the systems named (windows, macos, linux), or all three if none
# is; the others keep what the committed notice says for them, as a release
# for one system leaves the others' installers as they were. The serial is
# the committed one's plus one. Then commit it, apps/website/make.sh and
# deploy.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo="$here/../.."
out="$repo/apps/website/site/version.json"
secret=${OURSTICK_RELEASE_KEY:?set it to the release key file (the secret one)}
[ $# -ge 2 ] || { echo "usage: $0 <newest> <security> [windows|macos|linux...]" >&2; exit 2; }
newest=$1 security=$2
shift 2
systems=("$@")
[ ${#systems[@]} -gt 0 ] || systems=(windows macos linux)
for system in "${systems[@]}"; do
  case "$system" in windows | macos | linux) ;; *) echo "not a system: $system" >&2; exit 2 ;; esac
done
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
python3 -I - "$out" "$serial" "$made" "$newest" "$security" "${systems[@]}" > "$work/notice" <<'PY'
import json, os, sys
out, serial, made, newest, security, *systems = sys.argv[1:]
old = {}
if os.path.exists(out):
    old = json.loads(json.load(open(out))["notice"])
notice = {"serial": int(serial), "made": made}
for system in ("windows", "macos", "linux"):
    if system in systems:
        notice[system] = {"newest": newest, "security": security}
    elif system in old:
        notice[system] = old[system]
    else:
        sys.exit(f"no earlier notice for {system}: name it")
print(json.dumps(notice, indent=1), end="")
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
echo "Made $out: serial $serial, newest $newest, security $security, for ${systems[*]}"
