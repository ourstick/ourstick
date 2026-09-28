# Local database schema

_Updated with every migration. The source of truth is
`crates/ourstick-core/migrations/`; this file explains it._

Each device keeps its state in one SQLite database. **No secrets are stored in
it**: no signing keys, no StickKeys, no node secret key, no tickets. Those live
in the secret store (Secret Service or key file, see "Key storage" in
`docs/protocol.md`), under names derived from the StickId, so the database needs
no column for them. It only records *which* secret store this device uses.

## Conventions

- All tables are `STRICT`, with `CHECK` constraints for lengths and ranges.
- Keys and hashes are raw `BLOB`s (32 bytes), not hex.
- Times are Unix milliseconds (`INTEGER`), supplied by the caller's clock.
- `seq` values are stored as `INTEGER`, which is signed 64-bit. Values above
  2^63 − 1 are rejected by the code. `seq` is chosen as
  `max(last + 1, unix_time_ms)` (ADR 0004), which stays far below that.
- Paths are `TEXT` and must be valid UTF-8; other paths are rejected.
- The schema version is `PRAGMA user_version`. Migration `NNNN_*.sql` brings the
  database from version `NNNN − 1` to `NNNN`. A database whose version is
  newer than the code knows is refused ("made by a newer OurStick"), never
  modified.
- Connections set `PRAGMA foreign_keys = ON` and a busy timeout; file databases
  use `journal_mode = WAL`.

## Version 1 (`0001_initial.sql`)

### `device`

This device. At most one row (`id = 1`). Created once and never replaced:
replacing it would mean a new node key, which kills every ticket already shared
(CLAUDE.md rule 9).

| Column           | Type    | Meaning                                                  |
|------------------|---------|----------------------------------------------------------|
| `id`             | INTEGER | Always 1                                                 |
| `node_id`        | BLOB 32 | iroh NodeId (public key) of this device                  |
| `secret_backend` | TEXT    | `secret-service` or `key-file`                           |
| `key_file_path`  | TEXT    | Path of the key file; set if and only if `key-file`      |
| `created_at_ms`  | INTEGER | When the device was set up                               |

If a secret that the database says exists is missing from the recorded
backend, that is an error for the user to resolve, never a reason to generate
new keys.

### `owned_sticks`

Sticks this device owns. One row per stick.

| Column            | Type     | Meaning                                                    |
|-------------------|----------|------------------------------------------------------------|
| `stick_id`        | BLOB 32  | StickId (Ed25519 verifying key). Primary key.              |
| `name`            | TEXT     | Name shown to the owner                                    |
| `root_path`       | TEXT     | The shared folder                                          |
| `cipher`          | INTEGER  | `CipherId` byte (0 = none, 1 = XChaCha20-Poly1305)         |
| `last_signed_seq` | INTEGER  | Highest `seq` ever signed; 0 if none. Only increases.      |
| `latest_record`   | BLOB 137 | The latest signed manifest record, or NULL                 |
| `created_at_ms`   | INTEGER  | When the stick was created                                 |

`last_signed_seq` and `latest_record` change together, in one statement that
only succeeds if the new `seq` is greater than the stored one. Two processes
therefore cannot sign with the same `seq` and both record it, and a restored
old database cannot move `seq` back (ADR 0004 covers the clock term).

### `joined_sticks`

Sticks this device reads. One row per stick, created from a ticket.

| Column                  | Type    | Meaning                                                   |
|-------------------------|---------|-----------------------------------------------------------|
| `stick_id`              | BLOB 32 | StickId from the ticket. Primary key.                     |
| `output_dir`            | TEXT    | Where files are written                                   |
| `highest_seq`           | INTEGER | Highest accepted record `seq`; NULL before the first one  |
| `current_manifest_hash` | BLOB 32 | Manifest hash of that record; NULL together with `highest_seq` |
| `joined_at_ms`          | INTEGER | When the ticket was added                                 |

`highest_seq` is the `last_seen` of ADR 0004. Accepting a record updates both
columns in one statement that only succeeds if the new `seq` is greater than
the stored one (or the stored one is NULL).

## Not yet in the schema

- **Transfers** (per-file download progress for resume). iroh-blobs keeps
  partial blob data itself; what OurStick must add on top is decided with the
  transport (step 5) and the CLI (step 6), in a later migration.
- **Manifests.** Where decoded or encrypted manifests are cached is decided with
  manifest encryption.
