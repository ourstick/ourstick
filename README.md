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

```
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo run -p ourstick-cli
```

## License

Apache-2.0. See [LICENSE](LICENSE).

OurStick and the OurStick logo are trademarks of Giorgos. Use of the name for
forks or derived products requires permission.
