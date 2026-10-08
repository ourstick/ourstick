#!/bin/bash
# Builds OurStick.app and its .dmg on a Mac, signed with a Developer ID,
# notarized by Apple and stapled, so that it opens on any Mac without a
# warning and keeps its keychain access across updates.
#
# Run it in the Mac's own Terminal, not over ssh: the signing key and the
# notary password are in the login keychain. Needs Xcode's command line
# tools, Rust and `cargo install tauri-cli --version "=2.12.0" --locked`.
# The frontend (apps/app/dist) is built elsewhere (`npm run build`) and
# copied in: this Mac needs no Node.
#
#   OURSTICK_SIGNING_IDENTITY="Developer ID Application: NAME (TEAM)" \
#     tools/macos/make-dmg.sh
#
# OURSTICK_NOTARY_PROFILE names the notarytool keychain profile (made once
# with `xcrun notarytool store-credentials`); "ourstick-notary" by default.
set -euo pipefail

identity=${OURSTICK_SIGNING_IDENTITY:?set it to the Developer ID Application identity}
profile=${OURSTICK_NOTARY_PROFILE:-ourstick-notary}
root=$(cd "$(dirname "$0")/../.." && pwd)

if [ ! -f "$root/apps/app/dist/index.html" ]; then
  echo "apps/app/dist is missing: build the frontend and copy it here first." >&2
  exit 1
fi

cd "$root/apps/app"
# Tauri signs the app with the hardened runtime and a secure timestamp,
# which notarization requires; the frontend is already built.
APPLE_SIGNING_IDENTITY="$identity" cargo tauri build --bundles app,dmg \
  --config '{"build":{"beforeBuildCommand":""}}'

dmg=$(ls -t "$root"/target/release/bundle/dmg/*.dmg | head -1)
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
