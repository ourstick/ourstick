# ADR 0021: On macOS, the keys in the data protection keychain

**Status:** proposed — 2026-10-08.

## Context

On macOS the app keeps every secret (each stick's signing seed, stick key and
node key, each received stick's link, and the device's older node key) as a
generic password in the user's login keychain, the "legacy" file-based
keychain (`apple-native-keyring-store`, feature `keychain`). Its access list
names the program that made an item; any other program, and the same one
signed differently, is asked about: "OurStick wants to use your
confidential information stored in "ourstick" in your keychain", once per
item. Seen on 2026-10-08 with the first build signed with the Developer ID:
14 questions in a row for 14 items, the user's password for each. A user
with a handful of sticks would think something is wrong, and one who clicks
"Deny" leaves the app unable to share or receive.

The data protection keychain (the iOS-style keychain, on macOS since 10.15)
has no access lists and never asks: an item belongs to the app's keychain
access group, which only a program signed by our team with that group in its
entitlements can read. `apple-native-keyring-store` has it as its `protected`
store. What it needs:

- The app signed with the `keychain-access-groups` entitlement
  (`HHL7R3Q3XB.com.ourstick.OurStick`), and so, for an app outside the Mac
  App Store, a **Developer ID provisioning profile** for the App ID
  `com.ourstick.OurStick` embedded in the bundle
  (`Contents/embedded.provisionprofile`): without one, macOS refuses to
  start an app whose entitlements name an access group.
- The `protected` feature of `apple-native-keyring-store` (a change to the
  root `Cargo.toml`, which needs Giorgos's approval).
- Nothing for the CLI: it cannot carry a provisioning profile, and on macOS
  it already uses a key file.

Rule 9 of CLAUDE.md: a key is never made again once a link was shared. The
keys already in the login keychain must be moved, not replaced.

## Decision

1. **New items go to the data protection keychain** on macOS, in the app's
   access group, readable only when the Mac is unlocked
   (`kSecAttrAccessibleWhenUnlocked`, the store's default), local to the
   device (never the iCloud-synchronized store: a stick's keys must not
   leave the computer that shares it). The `device.secret_backend` value
   becomes `protected-keychain` (a migration and `docs/schema.md`).
2. **Existing items move once**, at the first start of a version with this
   ADR, before any stick is shared or received: for each secret the
   database says exists, read it from the login keychain (the last time
   macOS asks, once per item; "Always Allow" is not needed), write it to
   the data protection keychain, read it back and compare; only when every
   item is there and equal, record `protected-keychain` and then delete
   the login keychain's items. Any failure leaves everything as it was
   (both copies, the old backend recorded) and says why; the next start
   tries again. A key is never made anew during the move.
3. **One item per stick** is not needed: the data protection keychain never
   asks, so the number of items costs nothing.
4. **If the entitlement is missing** (a build without the profile, e.g. a
   developer's own `cargo tauri dev`), the data protection keychain refuses
   with `errSecMissingEntitlement`: the app says so and does not fall back
   to the login keychain silently (CLAUDE.md: no silent fall back to a
   weaker store). A development build keeps the login keychain explicitly,
   by a setting that release builds do not have.

## What Giorgos does, once

In the Apple Developer account (Certificates, Identifiers & Profiles):

1. **Identifiers → +** → App IDs → App → description "OurStick", bundle ID
   explicit `com.ourstick.OurStick`; capabilities: none needed beyond the
   default (keychain sharing is implicit in the access group).
2. **Profiles → +** → Distribution → **Developer ID** → App ID
   `com.ourstick.OurStick` → the Developer ID Application certificate →
   download `OurStick_Developer_ID.provisionprofile`.
3. Put it at `~/ourstick-profiles/` on the Mac (not in the repo: it names
   the team and the certificate); `make-dmg.sh` embeds it.

## To verify on the Mac first

- That the signed app with the profile and the entitlement starts, reads and
  writes an item in the data protection keychain, and asks nothing.
- That an item survives an app update signed the same way.
- That the move (rule 2) on the Mac's own sticks keeps every link working.
- That it works on an Intel Mac too (the crate's docs speak of Apple
  Silicon; Apple's of macOS 10.15 and later).
- What happens to a stick's keys when the Mac is locked while sharing
  (`WhenUnlocked`): sharing goes on with keys already in memory; a restart
  while locked waits for the unlock.

## Tests

The move as a function over two `SecretStore`s (fakes): every item copied
and checked before the old ones go; a failure halfway leaves both and the
old backend; a second run after a failure finishes; no key made. The
backend's name in the store (migration). The rest by hand on the Mac.

## Alternatives considered

- **One login-keychain item per stick (its keys together).** Fewer
  questions (one per stick), still questions after every update signed
  differently, and an item format to migrate anyway.
- **"Always Allow" and nothing else.** It holds only while the signature
  stays the same; the first signed build already asked 14 times.
- **A key file on macOS for the app too.** A passphrase to type at every
  start, which the keychain exists to spare.
