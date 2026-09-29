# OurStick

**A USB stick you can send.**

Drop folders into a stick, send a link or QR code, and the other person sees
and downloads the files on their PC or phone — as if you handed them a USB stick.
Peer-to-peer, end-to-end encrypted, no accounts, no size limits.

> Early development. The command-line tool works; the apps do not exist yet.

## Status

- [x] Core library (manifest, cipher, ticket, store, transport)
- [x] CLI: `ourstick share` / `ourstick ls` / `ourstick get`
- [x] The stick follows changes to the shared folder
- [ ] Desktop app (Linux, Windows, macOS)
- [ ] Android
- [ ] iOS

## Using the CLI

```
ourstick share <folder>                  # prints a link and a QR code, then serves
ourstick ls '<link>'                     # lists the files
ourstick get '<link>' [--out <folder>]   # downloads them; run again to resume
ourstick get '<link>' --follow           # ... and every new version as it comes
```

While `share` runs, changes to the folder are shared as they happen; only
files that changed are encrypted again (judged by size and modification
times; `share --rescan` reads every file). A later `get` fetches only what
changed. It never overwrites a file you changed in its folder, and keeps files
that left the stick.

The link carries the key: anyone who has it can read the stick. Data goes
directly between the two devices when possible. Up to 100 MB of a download may
go through a public relay while a direct path is looked for; after that, `get`
asks first (`--allow-relay` says yes in advance).

`share` keeps its keys in the Secret Service (GNOME Keyring, KWallet). Where
there is none (macOS for now, SSH sessions, servers), use
`ourstick share --key-file <path> <folder>` once; later runs use that file and
ask for its passphrase. Losing the keys or the passphrase ends every link
shared from that device.

## Development

Requires Rust 1.91 or newer (`rust-version` in `Cargo.toml`). The floor comes
from our dependencies, not from our own code:

- `iroh` 1.x and `iroh-blobs` (networking and blob transfer) need 1.91.
- `zbus-secret-service-keyring-store` (the Linux Secret Service backend for
  keeping keys) needs 1.88.
- `ed25519-dalek` 3 (stick signing keys), `argon2` 0.6 (key files) and
  `rpassword` 7 (reading key-file passphrases) need 1.85.

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -p ourstick-cli
```

## License

Apache-2.0. See [LICENSE](LICENSE).

OurStick and the OurStick logo are trademarks of Giorgos. Use of the name for
forks or derived products requires permission.
