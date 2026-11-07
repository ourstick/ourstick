# Architecture

The rules are in `CLAUDE.md`; the reasons for each design choice in
`docs/decisions/`; the formats on the wire and on disk in `docs/protocol.md`
and `docs/schema.md`; the desktop app in `docs/gui-spec.md`. This file is
the map.

```
crates/ourstick-core   all the logic; no UI, no Tauri, no CLI
crates/ourstick-cli    share / get / ls, a thin shell over the core
apps/app               the desktop app: Tauri 2 (Rust commands) + Svelte UI
apps/website           the static site, ourstick.com
third_party/           a patched copy of one dependency (ksni), until an
                       upstream release has the fix
```

Both shells call only `ourstick-core`; the core imports nothing from them.

## The core

Everything outside the process goes through a trait, so that tests run with
in-memory versions, without a network or a disk:

| Trait | What it stands for | Real one |
|---|---|---|
| `Publisher`, `Fetcher` | the network, owner's and reader's side (`transport`) | iroh, `transport/iroh/` |
| `Store` | local state (`store`) | SQLite, migrations in `migrations/` |
| `SecretStore` | keys (`secrets`) | the system keyring, or a key file |
| `Cipher` | encryption of blobs (`cipher`) | XChaCha20-Poly1305 |
| `Clock` | time (`clock`) | the system clock |
| `Disks` | free space (`space`) | the system's file systems |

Only `transport/iroh/` talks to iroh; `manifest` takes iroh-blobs' chunk
size so that its outboards are the ones iroh checks.

**Sharing** (`share`): a pass walks the stick's folder and the folders linked
to it (`layout`, `linked`), hashes each file's ciphertext with BLAKE3
(`manifest`, `blob`), records what it read (`store`) and publishes a signed
version (`record`): the encrypted manifest and its record. Blobs are not
kept: the files keeper makes each one from its file when a reader asks, and
checks it against the manifest before it is sent (ADR 0017). Reading gives
way to other programs using the same disk (`disk`, ADR 0013). The watcher
(`watcher`) starts a new pass when the folder changes.

**Receiving** (`download`, `received`): a reader fetches the latest record,
checks its signature and sequence, fetches and decrypts the manifest, and
then the chosen files in batches, with resume, placing each one only once it
is whole and checked. It reports how far it is to the owner, signed and
encrypted (`report`, ADR 0015).

**Tickets** (`ticket`): the public part (stick, node, address hints) and the
secret part (the stick's key), in the link's fragment only (ADR 0022).

## The shells

The CLI (`crates/ourstick-cli`) parses arguments, prints, and keeps its keys
in the system keyring or a key file.

The app (`apps/app/src-tauri/src`) adds what only a desktop has: the stick's
folder as a drive (`drive`: a bookmark on Linux, a disk image on macOS, a
drive letter on Windows), the tray (`tray`), the windows and their commands,
start at login (`login`), and the receiving window (`receiving`). The UI
(`apps/app/src`) calls Tauri commands only.
