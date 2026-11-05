# Release keys

Two [minisign](https://jedisct1.github.io/minisign/) public keys. Their
secret halves were made on 2026-11-05 and are kept by Giorgos, off every
computer that builds OurStick; no build machine and no CI job can sign.

- `ourstick-release.pub` (key id `2DDEFEB70EAC0BBB`) signs each release's
  SHA-256 sums.
- `ourstick-recovery.pub` (key id `971E8F0C5AA6A65E`) is kept on paper and
  never used, unless the release key is lost or stolen: then it signs the
  word that the release key changed.

Signing: `tools/release/sign.sh`.

## Checking a download

With the installer and `SHA256SUMS.txt` with its `SHA256SUMS.txt.minisig`
from https://ourstick.com/downloads/ (or `SHA256SUMS-<version>.txt` with
its `.minisig` from a GitHub Release), in one folder:

```sh
minisign -Vm SHA256SUMS.txt -P RWS7C6wOt/7eLZ1c4gP+CYXHWKYycOKldo6+LFUSHsA5TUwdiBot58Yh
sha256sum --ignore-missing -c SHA256SUMS.txt      # macOS: shasum -a 256 --ignore-missing -c
```

The first line says `Signature and comment signature verified`, the second
`OK` for the installer. Take the key from here or from the README, not
from the site that served the files: a site that could change the files
could change a key it shows.
