# OurStick

**A USB stick in a link.**

Put folders on a stick in the OurStick app and send its link. The
other person opens it in OurStick, picks what they want, and downloads it,
straight from your computer to theirs. Peer-to-peer, end-to-end encrypted, no
accounts, no size limits.

> Early development. Download the desktop app at https://ourstick.com.

## Status

- [x] Core library (manifest, cipher, ticket, store, transport)
- [x] CLI: `ourstick share` / `ourstick ls` / `ourstick get`
- [x] The stick follows changes to the shared folder
- [x] Desktop app (Linux, Windows, macOS on Apple silicon)
- [ ] Android
- [ ] iOS

## Using the CLI

```
ourstick share <folder>                  # prints a link and a QR code, then serves
ourstick ls '<link>'                     # lists the files
ourstick get '<link>' [--out <folder>]   # downloads them; run again to resume
ourstick get '<link>' --follow           # ... and every new version as it comes
ourstick get - [--out <folder>]          # the link asked for (not shown), or piped in
```

A link given as an argument can be read by every user of the computer (`ps`),
and it carries the stick's key: on a shared computer, give `-` and paste it
when asked, or `ourstick get - < link.txt`.

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
Keyring, KWallet) on Linux, the keychain on macOS, the Credential Manager on
Windows. Where there is none (SSH sessions, servers), use
`ourstick share --key-file <path> <folder>` once; later runs use that file and
ask for its passphrase. Losing the keys or the passphrase ends every link
shared from that device.

## Development

OurStick is written with Claude (Anthropic) as a programming partner, as the
commits say. [CLAUDE.md](CLAUDE.md), in Greek, holds the project's rules for
it; the decisions behind the design are in [docs/decisions](docs/decisions),
each reviewed before it was built.

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

Apache-2.0. See [LICENSE](LICENSE). The licence covers the code; it gives no
right to use the name OurStick or its logo (section 6 of the licence).
