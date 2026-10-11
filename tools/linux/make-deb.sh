#!/bin/bash
# Builds the Linux package, OurStick_<version>_amd64.deb, named `ourstick`.
#
# Tauri names a .deb after the product name in kebab case ("OurStick" ->
# `our-stick`), with no setting to change it. The package is unpacked,
# renamed in its control file and packed again; it declares that it
# replaces `our-stick` (0.1.0), so that installing it removes that one.
#
#   tools/linux/make-deb.sh
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
name=ourstick
old_name=our-stick

cd "$root/apps/app"
npm run tauri build -- --bundles deb
version=$(grep -m1 '^version' "$root/Cargo.toml" | cut -d'"' -f2)
deb="$root/target/release/bundle/deb/OurStick_${version}_amd64.deb"

work=$(mktemp -d)
trap 'rm -rf "${work:?}"' EXIT
dpkg-deb -R "$deb" "$work/package"
control="$work/package/DEBIAN/control"
grep -qx "Package: $old_name" "$control" || {
  echo "$deb is not the package $old_name; nothing renamed." >&2
  exit 1
}
sed -i "s/^Package: $old_name\$/Package: $name/" "$control"
cat >> "$control" <<EOF
Conflicts: $old_name
Replaces: $old_name
Provides: $old_name
EOF
dpkg-deb --root-owner-group -Zxz --build "$work/package" "$deb"
dpkg-deb -f "$deb" Package Version Conflicts Replaces Provides
echo "$deb"
