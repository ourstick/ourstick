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

## Version 5

Version 5 (`0005_drive_size.sql`) added `owned_sticks.drive_size`. Version 4
(`0004_downloaded_files.sql`) added `downloaded_files`, and version 3
(`0003_owned_files.sql`) added `owned_files`, both below.

Version 1 (`0001_initial.sql`) created the other tables. Version 2
(`0002_record_floors.sql`) moved the seq floor of ADR 0004 out of
`joined_sticks` into `record_floors`, so that `ls` can raise it without
touching folder state, and made `joined_sticks` say only what its folder holds.
The old `highest_seq`/`current_manifest_hash` were set before the files arrived,
so the migration copies them to `record_floors` and starts every
`completed_*` as NULL.

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
| `drive_size`      | INTEGER  | Size in bytes of a drive (the app's stick); NULL for a plain shared folder. A drive's folder must carry `.ourstick-drive` and may not hold more than this. |

`last_signed_seq` and `latest_record` change together, in one statement that
only succeeds if the new `seq` is greater than the stored one. Two processes
therefore cannot sign with the same `seq` and both record it, and a restored
old database cannot move `seq` back (ADR 0004 covers the clock term).

### `joined_sticks`

Sticks this device downloads, and what their folder holds. One row per stick,
created by the first `get`.

| Column                    | Type    | Meaning                                                       |
|---------------------------|---------|---------------------------------------------------------------|
| `stick_id`                | BLOB 32 | StickId from the ticket. Primary key.                         |
| `output_dir`              | TEXT    | The folder of the latest `get`                                |
| `completed_seq`           | INTEGER | `seq` of the record whose files are all in `output_dir`; NULL until a `get` finishes there |
| `completed_manifest_hash` | BLOB 32 | Manifest hash of that record; NULL together with `completed_seq` |
| `joined_at_ms`            | INTEGER | When the stick was first downloaded                           |

The latest folder wins: a `get` into another folder sets `output_dir` to it and
clears `completed_*`, because the new folder holds nothing yet. The old folder
is left as it is. `completed_*` is set only after every file is in place, and
only for a record at or below the floor in `record_floors`. A `get` reports
"no changes" only when `completed_*` names the record it just verified.

### `record_floors`

The highest verified record per stick: the `last_seen` of ADR 0004. Written by
every command that verifies a signed record (`get` and `ls`), whether or not
the stick has a folder.

| Column          | Type    | Meaning                                  |
|-----------------|---------|------------------------------------------|
| `stick_id`      | BLOB 32 | StickId. Primary key.                    |
| `highest_seq`   | INTEGER | Highest accepted record `seq`            |
| `manifest_hash` | BLOB 32 | Manifest hash of that record             |

Accepting a record inserts the row, or raises it in the same statement only if
the new `seq` is greater than the stored one. A record at or below the floor is
refused, so a restored or replayed old record cannot roll a reader back.

### `owned_files`

What the owner last encrypted, per file of an owned stick, so that an unchanged
file is not encrypted again. Replaced as a whole, in one transaction, after
every publish.

| Column      | Type    | Meaning                                                    |
|-------------|---------|------------------------------------------------------------|
| `stick_id`  | BLOB 32 | The owned stick (foreign key to `owned_sticks`)            |
| `path`      | TEXT    | Path relative to the stick root, as in the manifest        |
| `size`      | INTEGER | File size in bytes when it was last read                   |
| `mtime_ns`  | INTEGER | Modification time, ns since the Unix epoch; 0 if unknown   |
| `ctime_ns`  | INTEGER | Inode change time on Unix, ns; 0 elsewhere                 |
| `blob_hash` | BLOB 32 | The blob made from the file in that state                  |

Primary key `(stick_id, path)`. A file whose size, mtime and ctime match its
row, and whose blob the cache still holds, keeps `blob_hash` without being
read. This is a heuristic with a known limit ("Known limits" in
`docs/protocol.md`); `share --rescan` ignores the table. The stamp is taken
before the file is read, so a file that changes afterwards has a newer stamp
than its row and is read again next time.

### `downloaded_files`

What `get` wrote into a joined stick's folder, per file, so the next `get`
fetches only what changed and never overwrites a file changed in the folder.

| Column      | Type    | Meaning                                                    |
|-------------|---------|------------------------------------------------------------|
| `stick_id`  | BLOB 32 | The joined stick (foreign key to `joined_sticks`)          |
| `path`      | TEXT    | Path relative to the folder, as in the manifest            |
| `size`      | INTEGER | The file right after `get` wrote it: size in bytes         |
| `mtime_ns`  | INTEGER | … modification time, ns since the Unix epoch               |
| `ctime_ns`  | INTEGER | … inode change time on Unix, ns; 0 elsewhere               |
| `blob_hash` | BLOB 32 | The blob it was decrypted from                             |

Primary key `(stick_id, path)`. A row is written after each file is in place,
so an interrupted `get` keeps what it finished. Moving the stick to another
folder deletes its rows (in the same transaction as the move). Rows of files
that left the stick stay, so their files are known as `get`'s own.

## Not yet in the schema

- **Transfers** (per-file download progress for resume). iroh-blobs keeps
  partial blob data itself; what OurStick must add on top is decided with the
  transport (step 5) and the CLI (step 6), in a later migration.
- **Manifests.** Where decoded or encrypted manifests are cached is decided with
  manifest encryption.
