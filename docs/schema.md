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

## Version 28

Version 29 (`0029_version_look.sql`, ADR 0033) added the tables
`version_look` and `notice_serials`, below. A device set up before has a
`version_look` row with `looking_told` 0, so it is told once that the app now
looks for new versions; a fresh database gets the row with `looking_told` 1.

Version 28 (`0028_android_keystore.sql`, ADR 0027) lets
`device.secret_backend` be `android-keystore`, the app's store in the Android
Keystore; the table is made again with the same rows.

Version 27 (`0027_at_top.sql`, ADR 0028 §2, amendment of 2026-10-20) added
`linked_entries.at_top`, 0 for every entry before.

Version 26 (`0026_simpler_window.sql`, ADR 0028) added
`owned_sticks.has_own_folder`, 1 for every stick before (each keeps its
folder and its drive), and `settings.still_open_told`, 0.

Version 25 (`0025_waiting_links.sql`, ADR 0025) added the table
`waiting_links`, empty for every database before.

Version 24 (`0024_settings.sql`, ADR 0024) added the table `settings`, empty
for every database before: an upgraded device starts at login, and is told
so once.

Version 23 (`0023_protected_keychain.sql`, ADR 0021) lets
`device.secret_backend` be `protected-keychain`, the data protection keychain
of the app on macOS, and adds `device.old_keys_left` (0 for every device
before); the table is made again with the same rows.

Version 22 (`0022_local_network_explained.sql`, ADR 0020) added
`device.local_network_explained` (0 for every device before, so an upgraded
Mac shows the step once).

Version 21 (`0021_no_cache_claims.sql`, ADR 0017) deleted the `space_claims` rows
of kind `cache`. The kind is still allowed, so no database is refused, and is
never written. No column changed.

Version 20 (`0020_reader_reports.sql`, ADR 0015) added `reader_reports`,
`device.receiver_name` (NULL for every device before) and
`joined_sticks.report_counter` (0 for every stick before).

Version 19 (`0019_left_out.sql`, ADR 0012) added `joined_left_out`, below.

Version 18 (`0018_saved_as.sql`, ADR 0010) added `downloaded_files.saved_as`,
equal to `path` for every row before, and its unique index.

Version 17 (`0017_received_sticks.sql`, step 7b) added `joined_sticks.kind`
(`cli` for every row before), `name`, `paused` and `forgetting`, below.
Version 16 (`0016_scratch_claims.sql`, step 7b) lets `space_claims.kind` be
`scratch`: a download's room for its batches on the data folder's disk, held
as its own row, so that several downloads at once each hold theirs; the
table is made again with the same rows, since SQLite cannot change a CHECK
constraint. Version 15 (`0015_stick_life.sql`, ADR 0009 §5) added `owned_sticks.image_path`,
`sharing_wanted` and `removing`, and `retired_sticks`, below. Version 14 (`0014_no_drive_size.sql`, ADR 0008) dropped
`owned_sticks.drive_size`: a stick has no fixed size, and the app's stick is
told by `kind`. Version 13 (`0013_space_claims.sql`, ADR 0008's notes) added
`space_claims`, below. Version 12 (`0012_linked_entries.sql`, ADR 0008) added
`linked_entries`, below. Version 11 (`0011_stick_node_keys.sql`, ADR 0009) added
`owned_sticks.node_id` (NULL for every stick made before) and
`owned_sticks.kind` (`app` for the sticks that have a `drive_size`, `cli` for
the others), and made `device.node_id` optional; the `device` table is made
again with the same row, since SQLite cannot drop NOT NULL. Version 10 (`0010_latest_partial.sql`) added `owned_sticks.latest_partial`
(ADR 0007). Version 9 (`0009_owned_plaintext_hash.sql`) added
`owned_files.plaintext_hash` (ADR 0007), NULL in rows written before.
Version 8 (`0008_drive_letter.sql`) added `owned_sticks.drive_letter`, the
letter Windows showed the app's stick at. Version 7
(`0007_credential_manager_backend.sql`) lets
`device.secret_backend` be `credential-manager` (Windows); like version 6,
it makes the table again, since SQLite cannot change a CHECK constraint.
Version 6 (`0006_keychain_backend.sql`) did the same for `keychain`
(macOS). Version 5 (`0005_drive_size.sql`) added
`owned_sticks.drive_size`. Version 4
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
(CLAUDE.md rule 9). Since version 11 a device made anew has no node key of its
own (`node_id` NULL): each stick has one (ADR 0009). A device made before keeps
its node key, which its older sticks use.

| Column           | Type    | Meaning                                                  |
|------------------|---------|----------------------------------------------------------|
| `id`             | INTEGER | Always 1                                                 |
| `node_id`        | BLOB 32 or NULL | iroh NodeId (public key) of the device's own node key, used by sticks made before version 11; NULL on a device made after |
| `secret_backend` | TEXT    | `secret-service` (Linux), `keychain` (macOS: the login keychain, the CLI's and the app's before ADR 0021), `protected-keychain` (macOS: the app's data protection keychain, ADR 0021), `credential-manager` (Windows), `android-keystore` (Android, ADR 0027) or `key-file`. The keys are read only from the store this names |
| `key_file_path`  | TEXT    | Path of the key file; set if and only if `key-file`      |
| `created_at_ms`  | INTEGER | When the device was set up                               |
| `receiver_name`  | TEXT or NULL | The name this device's receiver chose to show senders (ADR 0015): 1–40 characters, not blank, none invisible (checked by the store; the column checks only that it is not empty); NULL (the usual) shows only the kind of computer |
| `old_keys_left` | INTEGER | 1 while the login keychain still holds the copies of keys moved to the data protection keychain (ADR 0021), deleted at a later start once every key reads from the new store; only with `protected-keychain` |
| `local_network_explained` | INTEGER | 1 once the app showed, on macOS, the step that explains the Local Network permission before macOS asks (ADR 0020); only that it was shown, never the permission's state, which macOS keeps |

If a secret that the database says exists is missing from the recorded
backend, that is an error for the user to resolve, never a reason to generate
new keys.

### `settings`

The app's choices on this device (ADR 0024). At most one row (`id = 1`); no
row means the defaults. Not in `device`, whose row exists only once a first
key is written: these hold from the first start. The CLI ignores them.

| Column | Type | Meaning |
|--------|------|---------|
| `id` | INTEGER | Always 1 |
| `start_at_login` | INTEGER | 1 (the default): the app makes the system start it at login (the `Run` value, the LaunchAgent or the autostart entry); 0: it removes that |
| `start_at_login_told` | INTEGER | 1 once the window said that the app starts at login, and how to turn it off |
| `still_open_told` | INTEGER | 1 once the window, closed the first time, said that OurStick is still open and how to open it again (ADR 0028 §6) |

### `version_look`

What the desktop app keeps of its looks for a new version (ADR 0033). At
most one row (`id = 1`); no row means the defaults (looking on, nothing
known). Phones and the CLI never write it.

| Column | Type | Meaning |
|--------|------|---------|
| `id` | INTEGER | Always 1 |
| `looking` | INTEGER | 1 (the default): "Look for new versions" is on; 0: the app reads nothing |
| `looking_told` | INTEGER | 1 once the window said that the app now looks, and how to turn it off |
| `next_look_ms` | INTEGER | When the next look is due (0: at once) |
| `last_good_ms` | INTEGER | The last look that found a good notice, NULL if none |
| `failing_since_ms` | INTEGER | Since when every look failed, NULL after a good one |
| `failure` | TEXT | How the last failed look failed: `no-connection`, `refused` (an HTTP status other than 200) or `not-the-file` (not a good notice) |
| `newest` | TEXT | The newest version a notice named for this system (`X.Y.Z`); forgotten when looking is turned off |
| `security` | TEXT | The oldest safe version a notice named; kept when looking is turned off |
| `new_release_key` | TEXT | The id (16 hex) of a new release key the recovery key announced |
| `security_told` | TEXT | The `security` the window told of once |

### `notice_serials`

The highest serial taken per key that signed a notice (ADR 0033 rule 5): a
notice with a lower one is ignored. One row per key id (16 hex, as minisign
shows it).

| Column | Type | Meaning |
|--------|------|---------|
| `key_id` | TEXT | The signing key's id. Primary key |
| `serial` | INTEGER | The highest serial taken from it |

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
| `drive_letter`    | TEXT     | Windows: the drive letter (`A`–`Z`) the stick was last shown at with `subst`, asked for again after a restart so that the stick stays where the user saw it. NULL elsewhere. |
| `node_id`         | BLOB 32 or NULL | iroh NodeId of the stick's own node key (`stick/<hex>/node-key`, ADR 0009); unique. NULL for a stick made before version 11, which uses `device.node_id` |
| `kind`            | TEXT     | `app` (the app's stick: a drive) or `cli` (shared from the command line); which program lists it |
| `image_path`      | TEXT or NULL | macOS: the app stick's disk image, recorded once and never derived from its name or id |
| `sharing_wanted`  | INTEGER  | 1 unless the user chose "Stop sharing" (kept across restarts) |
| `has_own_folder`  | INTEGER  | 1 (every stick made before ADR 0028): `root_path` is the stick's own folder, which the app makes a drive (letter, bookmark, disk image) and marks. 0 (an app stick made since): `root_path` is an empty folder, `<data>/sent/<stick id hex>`, and the stick is only its linked entries; no drive, no marker |
| `removing`        | INTEGER  | 1 while the stick is being removed: every read but `removing_sticks` leaves it out, so it is never shared again; the one place that finishes removals takes its drive and marker away (its files stay, ADR 0028 §7; a stick without a folder of its own loses its empty folder), and deletes its secrets and rows |
| `latest_partial`  | INTEGER  | 1 if `latest_record` is of a partial manifest (the owner was still reading its folder, ADR 0007), so a restarted owner goes on publishing partial versions until a complete one; else 0 |

`last_signed_seq`, `latest_record` and `latest_partial` change together, in one statement that
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
| `kind`                    | TEXT    | The program that receives it: `cli` (`ourstick get`) or `app` (the receiver window) |
| `name`                    | TEXT    | Its name as the last verified manifest gave it; NULL before one |
| `paused`                  | INTEGER | 1 if the user paused it in the receiver window                |
| `forgetting`              | INTEGER | 1 while it is being forgotten: hidden, finished at the next start |
| `report_counter`          | INTEGER | The counter of its last report to the owner (ADR 0015), so the next is higher across restarts and a clock set back; not NULL, ≥ 0, default 0 |

The latest folder wins: a `get` into another folder sets `output_dir` to it and
clears `completed_*`, because the new folder holds nothing yet; but `get` never
touches an `app` stick (not even in its folder: paused there, it stays paused),
and the receiver window takes over a `cli` one. Forgetting deletes the row, its
`downloaded_files` and its space claims, never its `record_floors` row (ADR
0004) nor the files received. The old folder
is left as it is. `completed_*` is set only after every file is in place, and
only for a record at or below the floor in `record_floors`. A `get` reports
"no changes" only when `completed_*` names the record it just verified.

### `waiting_links`

Sharing links the app waits to receive (ADR 0025): pasted while their
sender was away, received by themselves once the sender is back. Their
tickets are in the secret store, never here.

| Column        | Type    | Meaning                                  |
|---------------|---------|------------------------------------------|
| `stick_id`    | BLOB 32 | StickId from the ticket. Primary key.    |
| `added_at_ms` | INTEGER | When the user chose to wait; a wait ends 30 days after |

A stick is here or in `joined_sticks`, never both: a link is refused here
if its stick is received, and adding a received stick deletes its row
here in the same transaction.

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
file is not encrypted again. Written in parts as a pass goes (each part in one
transaction), so a pass stopped halfway keeps what it did; rows of files that
left the folder are removed at the end of a full pass.

| Column      | Type    | Meaning                                                    |
|-------------|---------|------------------------------------------------------------|
| `stick_id`  | BLOB 32 | The owned stick (foreign key to `owned_sticks`)            |
| `path`      | TEXT    | Path relative to the stick root, as in the manifest        |
| `size`      | INTEGER | File size in bytes when it was last read                   |
| `mtime_ns`  | INTEGER | Modification time, ns since the Unix epoch; 0 if unknown   |
| `ctime_ns`  | INTEGER | Inode change time on Unix, ns; 0 elsewhere                 |
| `blob_hash` | BLOB 32 | The blob made from the file in that state                  |
| `plaintext_hash` | BLOB 32 or NULL | BLAKE3 of the file in that state (the nonce prefix, ADR 0003; not encrypted: the database, in its 0700 folder, tells whoever reads it which known files a stick shares, so it is never copied elsewhere). The owner's store (ADR 0017) takes the nonce prefix from it, for the blob in `blob_hash` only, when it makes the blob from the file; every 16 KiB group is checked against the blob's outboard before it is sent. A file without it is not ready until the files keeper makes it. NULL in rows from before version 9 (the file is then read twice) |

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
| `path`      | TEXT    | Its path in the stick, as in the manifest                  |
| `saved_as`  | TEXT    | Where it was saved, relative to the folder: `path`, or the smallest change of it this computer can save (ADR 0010) |
| `size`      | INTEGER | The file right after `get` wrote it: size in bytes         |
| `mtime_ns`  | INTEGER | … modification time, ns since the Unix epoch               |
| `ctime_ns`  | INTEGER | … inode change time on Unix, ns; 0 elsewhere               |
| `blob_hash` | BLOB 32 | The blob it was decrypted from                             |

Primary key `(stick_id, path)`; unique `(stick_id, saved_as)`: two files of a
stick are never recorded at one place. From ADR 0010's step 4, a row of a
file that left the stick keeps its place taken while its file is there; once
the file is gone the row is forgotten, before a round chooses its places.
Rows are written in groups of up to 256 in one transaction (ADR 0014),
each only after its file is synced and in place, at the end of each batch
and on every way out of a round, so an interrupted `get` keeps what it
finished; a group lost to a power cut leaves files in place that the next
round finds equal by content and records again. Moving the stick to another
folder deletes its rows (in the same transaction as the move). Rows of files
that left the stick stay, so their files are known as `get`'s own.

### `joined_left_out`

What the user chose not to get of a received stick in the window (ADR
0012 and its Amendment 1): paths of one or two parts (`Films`,
`Disk/MUSIC`), byte-exact as in the stick's manifest; a file under one is
left out. Each part a plain name (none empty, `.` or `..`, no `\` or control
character); a path under another one in the set is not kept (both checked
by `download::LeftOut` on every save; rows from before, a one-part name,
are such paths, so no migration). A path not here comes, also one the
sender adds later. Replaced whole when the user
saves a choice; deleted with the stick when it is forgotten. `ourstick get`
never reads it.

| Column     | Type    | Meaning                                              |
|------------|---------|------------------------------------------------------|
| `stick_id` | BLOB 32 | The received stick (foreign key to `joined_sticks`)  |
| `name`     | TEXT    | A path of one or two parts, as above                 |

Primary key `(stick_id, name)`.

### `reader_reports`

The latest report of each reader of an owned stick (ADR 0015), as
`report::open` gave it: what the reader claims, never a proof. Written by
`Store::keep_report` only, which decides by `store::reports`:

- a report replaces its reader's row only if its `counter` is higher (an
  old report sent again changes nothing);
- a new reader is kept while the stick has fewer than 200 rows; else in
  place of the one silent longest if silent for 30 days, else of a hidden
  row (whose counter then goes, so an old report of that reader sent again
  could bring its line back until its next report); else refused (a flood
  fills only the free places);
- `forgotten` hides the row and keeps its counter;
- rows received more than 90 days ago are deleted, and never read (an old
  report of such a reader sent again is kept as new, until its next one);
- the owner's clock set back: `first_ms` is lowered to the report's time.

Deleted with the stick (removed, or a new link).

| Column        | Type    | Meaning                                                        |
|---------------|---------|----------------------------------------------------------------|
| `stick_id`    | BLOB 32 | The owned stick (foreign key to `owned_sticks`); length 32     |
| `reader_id`   | BLOB 32 | The reader's public key for this stick; length 32              |
| `counter`     | INTEGER | Its report's counter; ≥ 0                                      |
| `seq`         | INTEGER | The version it reports on; ≥ 0                                 |
| `state`       | TEXT    | `downloading`, `up_to_date`, `waiting`, `paused`, `forgotten`  |
| `percent`     | INTEGER | 0–99; set if and only if `downloading`                         |
| `reason`      | TEXT    | `sender_away`, `no_room`, `folder_missing`, `other`; set if and only if `waiting` |
| `everything`  | INTEGER | 1 if it gets the whole stick, else 0                           |
| `kind`        | TEXT    | `macos`, `windows`, `linux`, `other`                           |
| `name`        | TEXT    | A name its receiver typed (not empty), or NULL                 |
| `first_ms`    | INTEGER | When this owner first heard from it (its own clock); ≥ 0       |
| `received_ms` | INTEGER | When this owner received the report (its own clock); ≥ `first_ms` |
| `hidden`      | INTEGER | 1 if off the sender's list (removed, cleared, or forgotten by the reader); default 0 |

`STRICT, WITHOUT ROWID`. Primary key `(stick_id, reader_id)`; index
`(stick_id, received_ms)`.

### `linked_entries`

Folders and files shared where they are, not copied onto the stick (ADR 0008
§2). Each is a top-level entry of its stick: its manifest paths start with
`name`, except the one `at_top`, whose contents are the top level. Written by `ourstick_core::linked::plan_link`'s decision only.

| Column        | Type            | Meaning                                                  |
|---------------|-----------------|----------------------------------------------------------|
| `stick_id`    | BLOB 32         | The owned stick (foreign key to `owned_sticks`)          |
| `name`        | TEXT            | Its name on the stick; one path component, no `/`        |
| `path`        | TEXT            | Where it is on this device (absolute). Never leaves the device: a reader sees only `name` |
| `is_dir`      | INTEGER 0/1     | A folder (1) or a single file (0)                        |
| `disk`        | INTEGER or NULL | The disk it was on when linked (`st_dev`, the 64 bits stored as they are); NULL where the system has none. On another disk now: its disk is gone and an empty mount point is in its place |
| `added_at_ms` | INTEGER         | When it was linked                                       |
| `at_top`      | INTEGER 0/1     | 1: a folder whose contents are the stick's top level, without `name` in front (the one folder a stick was made from). At most one per stick, on a stick with `has_own_folder = 0`; never removed |

Primary key `(stick_id, name)`; names that differ only in letter case are kept
apart by `plan_link`, not by the database. Removing a row stops sharing the
entry and removes its `owned_files` rows (`name` and `name/…`) in the same
transaction; the user's files are never touched.

### `space_claims`

The space ledger: room a program holds on a disk for one stick, so that
every OurStick program of the device counts what the others hold (ADR 0008's
notes, `ourstick_core::space`).

| Column     | Type    | Meaning                                                     |
|------------|---------|-------------------------------------------------------------|
| `stick_id` | BLOB 32 | The stick (joined for a download; no foreign key) |
| `kind`     | TEXT    | `cache`: a sharer's cache budget, held whole (never written since ADR 0017; old rows deleted by version 21); `download`: a download's bytes still to come, on its folder's disk; `scratch`: a download's room for its batches, on the data folder's disk |
| `disk`     | TEXT    | Which disk: `dev:<st_dev>` on Unix, `vol:<volume>` on Windows |
| `bytes`    | INTEGER | How much it holds                                           |

Primary key `(stick_id, kind)`: a new claim replaces the old one. A row
counts only while its holder's lock is held (`download-<hex>.lock` for `download` and `scratch`); otherwise it is stale,
ignored, and replaced next time. Claims are decided and written in one
transaction (a download's two at once), so two programs never both take the
same room. A download gives its rows back when it returns: a stick that is up
to date holds none.

### `retired_sticks`

Stick ids replaced by "Give it a new link" (ADR 0009 §5). The new row took
the old one's place, folder, settings and linked entries in the same
transaction that wrote this row; what is left for a retired id is only its
secrets (never the device's `node-key`) and its `cache/<hex>` folder, so
cleaning it up never touches a folder of the user's.

| Column          | Type    | Meaning                              |
|-----------------|---------|--------------------------------------|
| `stick_id`      | BLOB 32 | The retired id (primary key)         |
| `replaced_by`   | BLOB 32 | The id that took its place           |
| `retired_at_ms` | INTEGER | When                                 |

A retired id is forgotten (its secrets and cache deleted) once the stick
that took its place, following new links given one after another, has its
drive marker naming it; while that stick's drive is not there (unplugged, or
a macOS image not attached), the id waits. Checked at start and whenever a
drive is mounted.

## Not yet in the schema

- **Transfers** (per-file download progress for resume). iroh-blobs keeps
  partial blob data itself; what OurStick must add on top is decided with the
  transport (step 5) and the CLI (step 6), in a later migration.
- **Manifests.** Where decoded or encrypted manifests are cached is decided with
  manifest encryption.
