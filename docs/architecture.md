# Architecture

See CLAUDE.md for the rules. This file describes the shape.

```
crates/ourstick-core     domain logic; traits: Transport, Store, Cipher
crates/ourstick-cli      share / ls / get
apps/app                 Tauri 2 — desktop + mobile
apps/website             static site
```

Data flow: folder → Manifest → Cipher → blobs → Transport (iroh) → Reader.
State: SQLite via Store. Tickets carry the key; keys never touch logs or the DB in plaintext.
