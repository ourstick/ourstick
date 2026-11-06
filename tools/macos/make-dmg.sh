#!/bin/bash
# Builds OurStick.app and its .dmg on a Mac, signed with a Developer ID,
# notarized by Apple and stapled, so that it opens on any Mac without a
# warning and keeps its keychain access across updates.
#
# Run it in the Mac's own Terminal, not over ssh: the signing key and the
# notary password are in the login keychain. Needs Xcode's command line
# tools, Rust and `cargo install tauri-cli --version "=2.12.0" --locked`.
# The frontend (apps/app/dist) is built elsewhere (`npm run build`) and
# copied in: this Mac needs no Node. `tools/macos/dist-hash.sh` where it was
# built gives OURSTICK_DIST_SHA256, and the app is signed only if the copy
# here has the same hash.
#
#   OURSTICK_SIGNING_IDENTITY="Developer ID Application: NAME (TEAM)" \
#   OURSTICK_DIST_SHA256=<dist-hash.sh's output> tools/macos/make-dmg.sh
#
# OURSTICK_NOTARY_PROFILE names the notarytool keychain profile (made once
# with `xcrun notarytool store-credentials`); "ourstick-notary" by default.
# OURSTICK_PROVISIONING_PROFILE is the Developer ID provisioning profile for
# com.ourstick.OurStick (ADR 0021; not in the repo), by default
# ~/ourstick-profiles/OurStick.provisionprofile. OURSTICK_NOTARIZE=0 builds
# and signs only, for a test on this Mac, and may go without
# OURSTICK_DIST_SHA256.
set -euo pipefail

identity=${OURSTICK_SIGNING_IDENTITY:?set it to the Developer ID Application identity}
profile=${OURSTICK_NOTARY_PROFILE:-ourstick-notary}
provisioning=${OURSTICK_PROVISIONING_PROFILE:-$HOME/ourstick-profiles/OurStick.provisionprofile}
notarize=${OURSTICK_NOTARIZE:-1}
app_id=HHL7R3Q3XB.com.ourstick.OurStick
root=$(cd "$(dirname "$0")/../.." && pwd)
# rustup's folder, if the shell's profile does not add it.
command -v cargo >/dev/null || export PATH="$HOME/.cargo/bin:$PATH"

if [ ! -f "$root/apps/app/dist/index.html" ]; then
  echo "apps/app/dist is missing: build the frontend and copy it here first." >&2
  exit 1
fi
dist_hash=$("$root/tools/macos/dist-hash.sh" "$root/apps/app/dist")
if [ -n "${OURSTICK_DIST_SHA256:-}" ]; then
  if [ "$dist_hash" != "$OURSTICK_DIST_SHA256" ]; then
    echo "apps/app/dist is not the frontend that was built (its hash is $dist_hash): copy it again." >&2
    exit 1
  fi
elif [ "$notarize" != 0 ]; then
  echo "Set OURSTICK_DIST_SHA256 to tools/macos/dist-hash.sh's output where the frontend was built." >&2
  exit 1
else
  echo "apps/app/dist not checked (no OURSTICK_DIST_SHA256): $dist_hash"
fi

# The profile allows the keychain entitlements (ADR 0021): without it, or
# with an expired one, macOS refuses to start the app.
if [ ! -f "$provisioning" ]; then
  echo "No provisioning profile at $provisioning (ADR 0021, \"What Giorgos does\")." >&2
  exit 1
fi
decoded=$(mktemp)
trap 'rm -f "$decoded"' EXIT
security cms -D -i "$provisioning" > "$decoded"
expires=$(/usr/libexec/PlistBuddy -c "Print :ExpirationDate" "$decoded")
# PlistBuddy writes the date in English; a Greek (or any) locale would not
# read "Mon Oct".
expires_at=$(LC_ALL=C date -j -f "%a %b %d %T %Z %Y" "$expires" +%s 2>/dev/null || true)
if [ -z "$expires_at" ]; then
  echo "The provisioning profile's expiry ($expires) could not be read." >&2
  exit 1
fi
if [ "$expires_at" -le "$(date +%s)" ]; then
  echo "The provisioning profile expired on $expires: make a new one." >&2
  exit 1
fi
if [ "$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.application-identifier" "$decoded")" != "$app_id" ]; then
  echo "The provisioning profile is not for $app_id." >&2
  exit 1
fi

cd "$root/apps/app"
# Tauri signs the app with the hardened runtime and a secure timestamp,
# which notarization requires, with the keychain entitlements and the
# profile inside the bundle; the frontend is already built.
config=$(/usr/bin/python3 -I -c 'import json, sys; print(json.dumps({"build": {"beforeBuildCommand": ""}, "bundle": {"macOS": {"entitlements": "OurStick.entitlements", "files": {"embedded.provisionprofile": sys.argv[1]}}}}))' "$provisioning")
bundles=app,dmg
[ "$notarize" = 0 ] && bundles=app
# No dmg of an earlier build may be taken for this one's.
rm -f "$root"/target/release/bundle/dmg/*.dmg
APPLE_SIGNING_IDENTITY="$identity" cargo tauri build --bundles "$bundles" --config "$config"

app="$root/target/release/bundle/macos/OurStick.app"
signed=$(codesign -d --entitlements :- "$app" 2>/dev/null || true)
if [ ! -f "$app/Contents/embedded.provisionprofile" ] ||
  ! grep -q "keychain-access-groups" <<<"$signed" ||
  ! grep -q "$app_id" <<<"$signed"; then
  echo "The app was signed without the keychain entitlements or its profile: not sent." >&2
  exit 1
fi
if [ "$notarize" = 0 ]; then
  echo "Signed, not notarized (OURSTICK_NOTARIZE=0): $app"
  exit 0
fi

dmg=$(ls "$root"/target/release/bundle/dmg/*.dmg | head -1)
[ -f "$dmg" ] || { echo "The build made no .dmg." >&2; exit 1; }
echo "Sending $(basename "$dmg") to Apple for notarization (a few minutes)..."
submitted=$(xcrun notarytool submit "$dmg" --keychain-profile "$profile" --wait 2>&1 | tee /dev/stderr)
if ! grep -q "status: Accepted" <<<"$submitted"; then
  id=$(sed -n 's/^ *id: //p' <<<"$submitted" | head -1)
  echo "Apple did not accept it. Why: xcrun notarytool log $id --keychain-profile $profile" >&2
  exit 1
fi
xcrun stapler staple "$dmg"

# What Gatekeeper will say on another Mac.
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
echo "Ready: $dmg"
