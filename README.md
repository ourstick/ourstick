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
directly between the two devices when possible. Without a direct path it goes
through a public relay, at up to 10 Mbps, while a direct path is still looked
for; a download never stops because of it.

`share` keeps its keys in the system keyring: the Secret Service (GNOME
Keyring, KWallet) on Linux, the keychain on macOS. Where there is none (SSH
sessions, servers, Windows for now), use
`ourstick share --key-file <path> <folder>` once; later runs use that file and
ask for its passphrase. Losing the keys or the passphrase ends every link
shared from that device.

## Development

Requires Rust 1.91 or newer (`rust-version` in `Cargo.toml`). The floor comes
from our dependencies, not from our own code:

- `iroh` 1.x and `iroh-blobs` (networking and blob transfer) need 1.91.
- `zbus-secret-service-keyring-store` (the Linux Secret Service backend for
  keeping keys) needs 1.88.
- `ed25519-dalek` 3 (stick signing keys), `argon2` 0.6 (key files),
  `rpassword` 7 (reading key-file passphrases) and `apple-native-keyring-store`
  (the macOS keychain) need 1.85.

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -p ourstick-cli
```

The desktop app (`apps/app`, Tauri 2 + Svelte) is part of the workspace, so the
commands above need its system libraries too. On Debian/Ubuntu:

```
sudo apt install libwebkit2gtk-4.1-dev librsvg2-dev \
    libxdo-dev libssl-dev build-essential file
```

On Linux the app's tray icon is a StatusNotifierItem; GNOME shows it through
the AppIndicator extension (on by default in Ubuntu).

macOS needs only the Xcode command line tools. The UI needs Node.js 22 or 24
(LTS):

```
cd apps/app
npm install
npm run tauri dev      # the app, reloading as the UI changes
npm run tauri build    # a release build, with the UI inside
npm run check          # type-check the UI
```

A release build made with plain cargo needs the UI built first (`npm run build`)
and `--features tauri/custom-protocol`; without it the app looks for the
development server and shows an empty window.

## License

Apache-2.0. See [LICENSE](LICENSE).

OurStick and the OurStick logo are trademarks of Giorgos. Use of the name for
forks or derived products requires permission.
