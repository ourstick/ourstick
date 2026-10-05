# Architecture

See CLAUDE.md for the rules. This file describes the shape.

```
crates/ourstick-core     domain logic; traits: Publisher/Fetcher (transport), Store,
                         SecretStore, Cipher, Clock
crates/ourstick-cli      share / ls / get
apps/app                 Tauri 2 — desktop + mobile
apps/website             static site
```

Data flow: folder → Manifest (with bao outboards) → blobs made from the files when
asked and checked before sending (Publisher, ADR 0017) → iroh → Fetcher → decrypt → reader's folder. Only `transport/iroh/` imports iroh
(ADR 0005). `share.rs` and `download.rs` hold the two flows.
State: SQLite via Store. Tickets carry the key; keys never touch logs or the DB in plaintext.
