#!/bin/bash
# Signs, with the code-signing key on a hardware token at this (Linux) host,
# the files that the Windows build asks for through
# tools/windows-sign/sign-in-vm.ps1. Run it yourself in a terminal while the
# build runs in the VM:
#
#   bash tools/windows-sign/sign-on-host.sh <settings file>
#
# The settings file, kept outside the repository, sets:
#   OSSLSIGNCODE     osslsigncode 2.16 or later (2.8 crashed in RSA_sign)
#   PKCS11_PROVIDER  OpenSSL's pkcs11-provider .so, 1.3.0 or later
#   PKCS11_MODULE    the token's PKCS#11 module
#   CERT_CHAIN       PEM: the code-signing certificate, then its issuer
#   KEY_URI          pkcs11:id=...;type=private (always with id=: the token
#                    holds other keys, and type= alone takes the wrong one)
#   TIMESTAMP_URL    the certificate authority's RFC 3161 server
#   VM_SSH_PORT, VM_SSH_HOST   the VM's ssh (key login)
#   CACHE_DIR        signed copies by the SHA-256 of what was signed: the
#                    NSIS plugins are the same in every build, so they ask
#                    for the PIN only the first time
#
# The token's PIN is asked by osslsigncode itself, once per file, and typed
# only by you. Nothing is retried: after a failure the script stops, so a
# wrong PIN is never tried twice.
set -euo pipefail
ulimit -c 0 # a crash dump could hold the PIN

[ $# -eq 1 ] || { echo "usage: $0 <settings file>" >&2; exit 2; }
# shellcheck source=/dev/null
. "$1"
for v in OSSLSIGNCODE PKCS11_PROVIDER PKCS11_MODULE CERT_CHAIN KEY_URI TIMESTAMP_URL VM_SSH_PORT VM_SSH_HOST CACHE_DIR; do
  [ -n "${!v:-}" ] || { echo "$v is not set in $1" >&2; exit 2; }
done
case "$KEY_URI" in *id=*) ;; *) echo "KEY_URI has no id=: the token holds more than one key" >&2; exit 2 ;; esac
version=$("$OSSLSIGNCODE" --version 2>&1 | head -1 | grep -oE '[0-9]+\.[0-9]+' | head -1)
printf '%s\n2.16\n' "$version" | sort -V -C -r ||
  { echo "osslsigncode $version is too old: 2.16 or later" >&2; exit 2; }
mkdir -p "$CACHE_DIR"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# -n: never read this terminal's input, which the PIN prompt needs.
vm() { ssh -n -o BatchMode=yes -p "$VM_SSH_PORT" "$VM_SSH_HOST" "$@"; }
from_vm() { scp -q -o BatchMode=yes -P "$VM_SSH_PORT" "$VM_SSH_HOST:ourstick-sign-queue/$1" "$2"; }
to_vm() { scp -q -o BatchMode=yes -P "$VM_SSH_PORT" "$1" "$VM_SSH_HOST:ourstick-sign-queue/$2"; }
fail() { # job, reason: tell the build in the VM, then stop
  printf '%s\n' "$2" > "$work/failed"
  to_vm "$work/failed" "$1/failed" || true
  echo "STOPPED: $2" >&2
  echo "Check the token without a PIN: pkcs11-tool --module $PKCS11_MODULE --list-token-slots" >&2
  exit 1
}

echo "Waiting for files from the VM (Ctrl+C to stop)."
while true; do
  # The queue folder is made here too: listing a missing one ends in exit 1.
  if ! waiting=$(vm '$q = Join-Path $env:USERPROFILE ourstick-sign-queue; New-Item -ItemType Directory -Force -Path $q | Out-Null; Get-ChildItem -Directory $q | Where-Object { (Test-Path (Join-Path $_.FullName ready)) -and -not (Test-Path (Join-Path $_.FullName done)) -and -not (Test-Path (Join-Path $_.FullName failed)) } | Sort-Object Name | ForEach-Object Name; exit 0' | tr -d '\r'); then
    echo "could not reach the VM over ssh; trying again in 10 s" >&2
    sleep 10
    continue
  fi
  for job in $waiting; do
    [[ "$job" =~ ^[0-9]{8}-[0-9]{6}-[0-9a-f]{8}$ ]] || { echo "skipping a strange job name" >&2; continue; }
    from_vm "$job/unsigned.bin" "$work/unsigned.bin"
    from_vm "$job/name.txt" "$work/name.txt"
    # The name only for the eye, and only in safe letters.
    name=$(tr -d '\r\n' < "$work/name.txt" | tr -c 'A-Za-z0-9._-' '_' | cut -c1-80)
    hash=$(sha256sum "$work/unsigned.bin" | cut -d' ' -f1)
    if [ -f "$CACHE_DIR/$hash" ]; then
      echo "$name: signed before, the same copy again"
      cp "$CACHE_DIR/$hash" "$work/signed.bin"
    else
      echo
      echo "Signing $name (sha256 $hash). Type the token's PIN once; if it asks again, press Ctrl+C."
      rm -f "$work/signed.bin"
      env PKCS11_PROVIDER_MODULE="$PKCS11_MODULE" "$OSSLSIGNCODE" sign \
        -provider "$PKCS11_PROVIDER" -certs "$CERT_CHAIN" -key "$KEY_URI" \
        -askpass -h sha256 -n OurStick -i https://ourstick.com \
        -ts "$TIMESTAMP_URL" -in "$work/unsigned.bin" -out "$work/signed.bin" ||
        fail "$job" "osslsigncode could not sign $name"
      cp "$work/signed.bin" "$CACHE_DIR/$hash.tmp" && mv "$CACHE_DIR/$hash.tmp" "$CACHE_DIR/$hash"
    fi
    to_vm "$work/signed.bin" "$job/signed.bin"
    printf '%s\n' "$hash" > "$work/done"
    to_vm "$work/done" "$job/done"
    echo "$name: back in the VM"
  done
  sleep 2
done
