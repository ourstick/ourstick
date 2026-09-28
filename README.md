# OurStick

**A USB stick you can send.**

Drop folders into a stick, send a link or QR code, and the other person sees
and downloads the files on their PC or phone — as if you handed them a USB stick.
Peer-to-peer, end-to-end encrypted, no accounts, no size limits.

> Early development. Nothing works yet.

## Status

- [ ] Core library (manifest, cipher, ticket, store, transport)
- [ ] CLI: `ourstick share` / `ourstick get`
- [ ] Desktop app (Linux, Windows, macOS)
- [ ] Android
- [ ] iOS

## Development

Requires Rust 1.88 or newer (`rust-version` in `Cargo.toml`). The floor comes
from our dependencies, not from our own code:

- `zbus-secret-service-keyring-store` (the Linux Secret Service backend for
  keeping keys) needs 1.88.
- `ed25519-dalek` 3 (stick signing keys) and `argon2` 0.6 (key files) need 1.85.

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -p ourstick-cli
```

## License

Apache-2.0. See [LICENSE](LICENSE).

OurStick and the OurStick logo are trademarks of Giorgos. Use of the name for
forks or derived products requires permission.
