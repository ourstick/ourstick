# OurStick protocol

_Draft. Updated whenever the wire format or the ticket format changes._

## Concepts

- **Stick** — a named, keyed collection of root folders.
- **StickId** — the stick's Ed25519 verifying key. The owner alone holds the
  signing key. Separate from the iroh NodeId (ADR 0004).
- **StickKey** — the 32-byte symmetric key that decrypts the stick.
- **Manifest** — index of all files: path, size, BLAKE3 hash of the encrypted blob.
- **Ticket** — everything a reader needs. Public part: StickId, NodeId, address
  hints, manifest seq and hash (a hint and a floor). Secret part: StickKey.
- **Signed record** — `(StickId, seq, manifest_hash)` signed by the owner. The
  latest one a reader accepted defines the current manifest.
- **Peer** — Owner, Reader or Seed. The protocol does not distinguish a Seed from any other peer.

## Flow

1. Owner catalogues its folder (every file's blob hash), signs a record for
   the manifest, and serves both over iroh; partial versions come out while
   the catalogue grows. A file's blob is made from the file when a reader asks
   for it, and checked before it is sent (ADR 0017).
2. Owner produces a ticket (link / QR).
3. Reader decodes the ticket and connects to the owner's NodeId (address hints
   first; iroh address lookup and relays otherwise).
4. Reader asks for the latest signed record (`ourstick/record/1`, below) and
   checks it against the ticket and what it has seen before.
5. Reader fetches the manifest blob by the record's hash, decrypts and
   validates it.
6. Reader fetches each file blob by hash (iroh-blobs, verified BLAKE3 streaming,
   resumable), decrypts it to a temporary file, and renames it into place.
   It does so in **batches** (ADR 0007): runs of manifest entries, cut from
   the whole manifest at 1 GiB or 512 files, each fetched into a blob store
   of its own that is deleted once every entry of the batch is in place, so
   the reader needs about the size of the files plus one batch. The manifest
   has a store of its own too (`manifest`). A second run of the same version
   finds the same batches and resumes the partial blob of the one it stopped
   in; batches of another version are deleted. An entry whose blob this
   download already turned into a file is copied from that file, not fetched
   again (identical files share a blob, ADR 0003). A file is decrypted into
   place while the next one is fetched; at most one waits to be placed, so
   the reader needs one more file's blob in space.

## Manifest wire format (version 2)

```
manifest_bytes = format_version:u8 || postcard(Manifest)
```

`format_version` is `2`. A reader also reads `1` (below) and rejects any other
value. Postcard is not self-describing, so any change to the fields below needs
a new version. An owner writes version 2, which readers older than it cannot
read: they say the version is unsupported instead of reporting a stick as
complete when it is not. Test vectors: `docs/test-vectors/manifest-v2.json`,
from an independent generator.

`Manifest`, fields in this order:

| Field     | Postcard type    | Meaning                                                    |
|-----------|------------------|------------------------------------------------------------|
| `partial` | bool (one byte)  | More files follow in a later version: the owner is still reading its folder (ADR 0007). A reader waits for a complete version. |
| `name`    | string           | The stick's name as its owner chose it (ADR 0009): the folder's name for a stick shared from the command line, the stick's name in the app. The owner drops the characters the rule below refuses, trims it and cuts it to 60 scalar values. Owners before this sent it empty, so a stick's first version after an upgrade is a new version (its manifest hash changes). |
| `entries` | sequence of `Entry` | Varint count, then the entries.                         |

Version 1 is the entries alone (varint count, then the entries), read as
complete and nameless.

Each `Entry`, fields in this order:

| Field   | Postcard type     | Meaning                                                   |
|---------|-------------------|-----------------------------------------------------------|
| `path`  | string            | Path relative to the stick root, `/`-separated            |
| `size`  | u64 (varint)      | Size of the file (plaintext) in bytes                     |
| `mtime` | u64 (varint)      | Modification time, whole seconds since the UNIX epoch (clamped to 0) |
| `hash`  | `[u8; 32]` (raw)  | BLAKE3 hash of the file's blob (ciphertext, header included) |

### Validity rules

A reader rejects the whole manifest if any rule fails. A writer never produces one that fails.

- Nothing follows the encoded `Manifest` (no trailing bytes).
- `name` has at most 60 Unicode scalar values, none a control, format, line or
  paragraph separator character (Unicode categories Cc, Cf, Zl, Zp). It is
  text from a peer: escaped before any terminal or window shows it.
- Entries are strictly sorted by `path`, compared as bytes, which rules out
  duplicates and makes the encoding canonical.
- Every `path` is non-empty, contains no `\` and no NUL, and splitting it on `/`
  gives no component that is empty, `.` or `..`. This rules out absolute paths
  and escaping the output folder.

The rules are platform-neutral: a manifest that follows them is valid on the
wire. A reader never refuses a manifest for a name its own system cannot
save (ADR 0010): it saves every file, and a name it cannot save as it is
gets the smallest change that makes it savable, chosen by the reader and
recorded, so later versions write to the same place:

- on Windows, `< > : " | ? *` and control characters become `_`, a trailing
  dot or space is dropped, a name Windows keeps for itself (`CON`, `PRN`,
  `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension)
  gets `_` after its stem;
- on every system, a name that looks like a Windows drive (an ASCII letter
  followed by `:`, e.g. `c:evil`) becomes `c_evil`, so a stick behaves the
  same everywhere; a first name the reader uses itself (any name starting
  with `.ourstick-`, such as `.ourstick-partial` and `.ourstick-received`,
  in any letter case) gets `_` in front, and an owner leaves such names out
  of its sticks; a name too
  long for the system is cut, keeping its extension; a name no rule can
  make savable (it should not happen) is saved as `_`;
- names that would be one file in the reader's folder (letter case, where
  that folder's disk ignores it; Unicode forms on macOS), or a file where a
  folder must go, get ` (2)`, ` (3)`… The place a file was saved first is
  kept in every later version; a renamed folder holds all its files. A copy
  under a changed name stays when the sender later renames the file, like
  any file no longer in the stick.

Nothing on the wire changes: the sender does not know. A reader refuses to
write through a symbolic link that exists in its output folder, checking
when a file starts and again right before it is moved into place; a
symbolic link where a folder of the stick must go is in the way like a file.

### What the manifest hash covers

The manifest hash is the BLAKE3 hash of the manifest blob as sent. All fields,
including `mtime`, are covered: a reader can trust every field it decodes.
Changing only a file's `mtime` therefore produces a new manifest, but not new
file blobs.

The manifest bytes (`manifest_bytes` above) are encrypted into a blob in the
format below, with the stick's cipher and key, exactly like a file. The
manifest hash is the BLAKE3 hash of that blob, and is what signed records and
the ticket name. Because the nonce prefix is derived from the content
(ADR 0003), the manifest hash stays the same while the manifest is unchanged,
and changes whenever any field changes.

A reader learns the stick's cipher from the manifest blob's header (the ticket
carries only the key). Every file blob of that stick must use the same cipher.

### Owner-side rules for building a manifest

- Symlinks are not followed and not included. Sockets, devices and other
  non-regular files are skipped. Empty directories are not recorded.
- A file name that is not UTF-8, or that breaks the path rules above, is an error.
- System files are never part of a stick, at any depth, as a file or as a
  folder: `.DS_Store`, `.Spotlight-V100`, `.fseventsd`, `.Trashes`,
  `.TemporaryItems`, `.DocumentRevisions-V100`, `.VolumeIcon.icns`, `.apdisk`,
  `.metadata_never_index`, `.com.apple.timemachine.donotpresent`, `Thumbs.db`,
  `desktop.ini`, `$RECYCLE.BIN`, `System Volume Information`, `.directory`,
  `.ourstick-drive`, `.ourstick-partial`, and names starting with `._` or
  `.Trash-`. The shared folder itself may have any name.
- A **drive** (the app's stick, `owned_sticks.kind = 'app'`) carries the file
  `.ourstick-drive`, holding its StickId in hex. Without it (an ejected disk
  image, an empty mount point, another stick's drive) the owner does not
  publish, and keeps serving the last version.
- A stick may have **linked entries** (ADR 0008): folders or files shared
  from where they are, each a top-level entry under its name (`Films/…`, or
  `notes.txt` for a file). Nothing about them is on the wire but the name;
  where they are never leaves the device. A top-level entry of the stick's
  own folder with a linked entry's name (ignoring case) is left out. A linked
  entry that is **gone** (missing, of another kind, or on another disk than
  when linked: an empty mount point) keeps its files in the version as last
  recorded, so the last version stays offered; readers asking for them are
  told "not ready" until it is back, and the owner tries again later (from
  1 s, doubling to a minute).
- A version changes only when the folder does: if the new manifest hash equals
  the one in the last signed record, that record is served again and no new
  `seq` is signed.
- The owner works in **passes** (ADR 0007). A pass takes the **catalogue**
  first, in parts, in manifest order: every file's blob hash, from a full
  encryption whose bytes are thrown away, and, for a blob of more than one
  16 KiB group, its bao outboard (see below). A file whose stamp is as recorded
  (see "Known limits" below) keeps its recorded hash without being read;
  `share --rescan` reads every file. A file gone or changing while read is
  left out of the version, and another pass follows soon. Each part is
  recorded (`owned_files`, with the plaintext hash), so a pass stopped
  halfway goes on from there.
- **Blobs are made when asked** (ADR 0017). The owner keeps no encrypted
  copy of a file. One store per stick makes a blob's bytes from the file as
  they are sent: the nonce prefix comes from the plaintext hash the
  catalogue recorded for that blob (ADR 0003), and the chunks are encrypted
  as `encrypt_blob` would. The blob's size, its root (the manifest's hash)
  and its outboard all come from the catalogue's record, never from the
  file. Before a transfer, the file must be there with its recorded size,
  and the outboard with its length; otherwise the blob is "not ready". Each
  16 KiB group is checked against the outboard before it is sent, and the
  first group that does not match is never sent: the transfer stops
  (`ERR_INTERNAL`), the blob is "not ready" from then on, and the file's
  record is forgotten so the next pass reads it again. So no byte that differs from
  the hashed blob ever leaves, and no nonce is used for two plaintexts
  outside the owner. A file shorter than its record is a read error; bytes
  past its recorded size are never read.
- **Outboards** are kept in the stick's cache folder as
  `outboards/<blob hash in hex>.obao` (iroh's pre-order bao outboard, 16 KiB
  groups, 64 bytes per group: about 0.4 % of the blob), written to a
  temporary file and renamed. A blob of at most one group has none. An
  outboard is used only if its length is the one the blob's size gives;
  otherwise its file is "not ready" until it is made again. Reading a file
  again replaces an outboard that differs from the one it gives (cut short
  or damaged). After a complete version, the pass removes the outboards of
  blobs no longer in it.
- A file needs its plaintext hash and, over one group, its outboard to be
  served. Without them (a cache from before ADR 0017, a cleared cache) its
  blob is "not ready". The owner's **files keeper** makes them: first for
  blobs readers wait on, then the rest of the stick in manifest order,
  through the disks' turns. Making one reads the file as it is now and keeps
  the result only if the blob hash it gives is the manifest's; otherwise the
  record is forgotten. There is no cache budget and no space claim for
  the owner. When sharing starts, old `cache/<stick>/suitcases` folders are
  removed.
- While a reader was told "not ready" within the last minute, the catalogue
  pauses after each part as long as the part took (at most 10 s): the files
  keeper's reads for waiting readers use the same disk and go first, and the
  catalogue still goes on.
- A long pass publishes **partial** versions: an empty one at once if the
  stick was never published (so it has a link), then after 30 seconds and
  every 3 minutes, each a new `seq`; the complete version comes at the end. A
  partial version lists the files read so far and, after them in manifest
  order, the files the last pass recorded that are still in the folder and
  not read yet, as they were then: a reader sees new files early, and no file
  leaves the stick only because the pass has not reached it (one that did
  leave is gone at once). A partial version that would list what the last
  one listed is not published, so a pass over an unchanged folder signs
  nothing new.
- After a start, the owner serves its latest signed version again at once,
  from the manifest kept in its cache folder (`latest-manifest.blob`, written
  at every publish), and then reads the folder in a pass. Only a stick never
  published, or one whose kept manifest is gone, waits for the pass.
- While serving, the owner watches the folder (inotify, FSEvents, …; every
  60 s if the system cannot report changes). After a sign of change it waits
  until the folder has been quiet for 2 s and no file's size or times change
  over 1 s, then runs a pass as above. Opening and reading files are not
  changes. A failed pass (e.g. an unreadable file), or one that left files
  out, is reported and tried again at the next change or after 30 s.
- The owner gives way to other programs using its disk (ADR 0013): every
  read of a stick's files by the catalogue or the files keeper goes through its
  disk's turns, one reader a window (64 MB or 64 files) at a time, and is
  timed; when windows take over three times their usual time twice in a
  row, the owner reads a tenth of the time until one takes under two and a
  half times. On Linux the kernel's counters of the physical disk decide
  instead (ADR 0013, amendment 1): someone else is there when, two windows
  in a row, others moved a quarter of what the owner took from the disk
  and at least 4 MB, or a slow window came with 64 operations (and two a
  file) the owner did not ask for. A file readers wait for goes first, three windows to
  one. Blobs sent are read through the turns too (ADR 0017, amendment 1):
  all transfers of a stick from one disk are one reader, which reads in
  one turn at once, for a window, and loses it to the next reader in the
  queue after 50 ms with no read in flight; its windows are judged as
  transfers', by the wall time of their reads. Their reads and encryption run on 4 threads for the whole
  program. Nothing on the wire changes: a reader may wait longer, told "not
  ready" as before.

### Reader-side rules for the output folder

- A file whose stamp is as `get` left it (`downloaded_files`) is fetched again
  only if the manifest names another blob for it.
- A file with another stamp, or one `get` never wrote, is compared by content:
  the reader encrypts it with the stick's key and compares blob hashes (the
  nonce is derived from the content, so equal content gives an equal hash).
  Equal: it is already here. Different: it was changed in the folder, and it
  is left as it is and reported, every time, until the user moves or deletes
  it; then the stick's version is written.
- Right before a file is moved into place, the file there is checked again:
  it must be absent, or still as planned. Otherwise it is left and reported.
- Each file is written into the hidden partial folder, synced, then moved
  into place; only then is it recorded. Small files (at most 4 MB) are
  placed up to 16 at once and 64 MB in all, a bigger one alone; records
  are written 256 to a transaction (ADR 0014). Nothing on the wire depends
  on this.
- Files that left the stick are kept and reported, never deleted.
- A reader may leave out entries of a stick (ADR 0012 and its Amendment 1):
  a set of paths of one or two parts (a top-level entry, or one inside a
  top-level folder), byte-exact as in the manifest, read at every round; a
  file is left out when its path is one of them or lies under one. Nothing
  left out is fetched, counted or given a new place; files of it already
  here stay where they were recorded and are no longer updated. New entries
  are fetched, inside a folder too. Nothing on the wire changes: the owner
  never learns what a reader leaves out.
- If the owner refuses a blob during a download, most likely because it
  published a new version and stopped serving the old blobs, the reader
  fetches the record again. With a newer version it goes on with that one
  (files already written count as already here); with the same version the
  refusal stands. At most 10 new versions are followed per download.
- If the owner answers a blob request with "not ready" (iroh-blobs'
  `ERR_LIMIT`: the file's plaintext hash or outboard is still being made,
  or its file is gone or changed, ADR 0017), it is not a
  failure. The reader says once per file that the sender is preparing it,
  and asks again after a pause that starts at 250 ms and doubles up to a
  minute. After 5 minutes of waiting on one file it reconnects and fetches
  the record again, following a newer version if there is one (this does not
  count against the 10). It never gives up on "not ready" alone; the user
  can stop it.
- If the owner ends a transfer with `ERR_INTERNAL` (a read error or a
  mismatch while it made the blob, ADR 0017), the reader treats the file as
  one that keeps changing: a short wait, then it asks again. It is not
  "damaged".
- A **partial** manifest (the owner is still reading its folder, ADR 0007)
  does not end a download: once every file it lists is in place, the reader
  asks for the owner's record every 15 seconds and goes on with the next
  version, until one is complete. These versions are not counted against the
  10 above. `ls` says when a listing is partial.
- A version counts as complete in a folder (`joined_sticks.completed_*`) only
  if it is not partial and every file of it is there; a file left as changed
  keeps it incomplete.

### Known limits

- **Unchanged-file detection is a heuristic.** The owner records, per file,
  the size, mtime and (on Unix) ctime in nanoseconds when it last read it
  (`owned_files`, `docs/schema.md`), and does not read a file whose stamp is
  unchanged in the catalogue. A change that keeps all three is not seen by
  the catalogue: the version keeps the old blob hash. When a reader asks for
  that file, the owner's check of each group against the outboard finds that the file
  no longer gives that blob, so the rest is not sent (the reader waits), and
  its record is forgotten:
  the next pass reads it and publishes the new content. On Unix, ctime changes on every write
  and cannot be set back by ordinary tools, so this needs a file system with
  coarse or missing timestamps, a clock set backwards, or a deliberate change of
  the system time. Elsewhere only size and mtime count, and a tool that
  restores mtime after writing (some sync and archive tools do) is enough.
  `ourstick share --rescan` ignores the record and reads every file.

## Blob format (version 1)

A blob is one file (or the manifest), encrypted and chunked. The BLAKE3 hash in
the manifest is taken over the whole blob, header included. Why the nonce is
derived and not random: `docs/decisions/0003-deterministic-nonce.md`.

```
blob = header || chunk_0 || chunk_1 || ... || chunk_(n-1)
```

### Header

| Offset | Length | Field          | Value                                             |
|--------|--------|----------------|---------------------------------------------------|
| 0      | 1      | `version`      | `0x01`. A reader rejects any other value.         |
| 1      | 1      | `cipher`       | `0x00` = none, `0x01` = XChaCha20-Poly1305. Others rejected. |
| 2      | 19     | `nonce_prefix` | Only for cipher `0x01`. Absent for cipher `0x00`. |

Header length: 2 bytes for cipher `0x00`, 21 bytes for cipher `0x01`.
A reader must know the stick's cipher and rejects a blob whose `cipher` differs.

### Chunks

The plaintext is split into chunks of `CHUNK_SIZE` = 65536 bytes. Every chunk
holds exactly 65536 plaintext bytes except the last, which holds 1..=65536.
An empty plaintext is one chunk of 0 bytes. A plaintext that is an exact
multiple of 65536 has no extra empty chunk: its last full chunk is the last one.
At most 2^32 chunks (256 TiB per file).

| Cipher | Stored chunk                       | Stored size       |
|--------|------------------------------------|-------------------|
| `0x00` | the plaintext chunk as is          | plaintext length  |
| `0x01` | ciphertext `\|\|` Poly1305 tag (16 B) | plaintext length + 16 |

Total blob length for cipher `0x01`: `21 + plaintext_len + 16 × max(1, ceil(plaintext_len / 65536))`.

A reader reads stored chunks of 65536 + 16 bytes. A shorter chunk, or the one
followed by end of blob, is the last. Because the last flag is inside the nonce,
truncating a blob at a chunk boundary, appending data, reordering chunks, or
moving a chunk from another blob all fail authentication.

### Keys (cipher `0x01`)

Both keys are derived from the 32-byte stick key with BLAKE3 in `derive_key` mode:

| Key         | BLAKE3 `derive_key` context string                   | Used for            |
|-------------|------------------------------------------------------|---------------------|
| `aead_key`  | `OurStick 2026-09-28 blob XChaCha20-Poly1305 key`    | XChaCha20-Poly1305 key |
| `nonce_key` | `OurStick 2026-09-28 blob nonce prefix key`          | Deriving nonce prefixes |

### Nonce prefix (cipher `0x01`)

```
nonce_prefix = first 19 bytes of BLAKE3-keyed(key = nonce_key, input = BLAKE3(plaintext))
```

`BLAKE3(plaintext)` is the 32-byte hash of the whole plaintext of the blob.

### Chunk nonce (cipher `0x01`)

The 24-byte XChaCha20 nonce of chunk `i` (counting from 0):

| Offset | Length | Field          | Value                                            |
|--------|--------|----------------|--------------------------------------------------|
| 0      | 19     | `nonce_prefix` | From the header                                  |
| 19     | 4      | `index`        | `i` as u32, big-endian                           |
| 23     | 1      | `last`         | `0x01` for the last chunk, `0x00` for all others |

### Chunk AAD (cipher `0x01`)

The associated data of every chunk is the full header: all 21 bytes, i.e.
`version || cipher || nonce_prefix`. A chunk therefore only opens under the exact
header it was sealed with.

### Test vectors

`docs/test-vectors/blob-v1.json` has vectors for both ciphers: key, plaintext,
derived keys, nonce prefix, first and last chunk nonce, header, blob length,
blob hash, and the full blob when it is at most 1024 bytes. They were generated
by `docs/test-vectors/generate_blob_v1.py`, an independent Python implementation
of this section. `crates/ourstick-core/tests/blob_vectors.rs` checks the Rust
implementation against them.

## Ticket (version 1)

### Text form

```
https://ourstick.com/s/<public>#<secret>
```

`<public>` and `<secret>` are the byte strings below in base32 (RFC 4648
alphabet, lowercase, no padding). Decoders accept either case, and also accept
the bare form `<public>#<secret>` without the `https://ourstick.com/s/` prefix.

The secret lives only in the URL fragment, which browsers never send to a
server. It never goes in the path or query. There is no custom URL scheme
(`ourstick://`) carrying a key: any app could claim such a scheme and receive it.
Tickets are never written to logs.

### Public part

| Offset | Length | Field     | Value                                               |
|--------|--------|-----------|-----------------------------------------------------|
| 0      | 1      | `version` | `0x01`. A newer value means "update OurStick".      |
| 1      | n      | `body`    | `postcard(PublicBody)`, see below                   |
| 1 + n  | 4      | `check`   | First 4 bytes of `BLAKE3(version \|\| body)`         |

`PublicBody`, postcard fields in this order:

| Field           | Postcard type         | Meaning                                         |
|-----------------|-----------------------|-------------------------------------------------|
| `stick_id`      | `[u8; 32]` (raw)      | Ed25519 verifying key of the stick              |
| `node_id`       | `[u8; 32]` (raw)      | iroh NodeId of the owner                        |
| `manifest_seq`  | u64 (varint)          | `seq` of the latest signed record at share time |
| `manifest_hash` | `[u8; 32]` (raw)      | `manifest_hash` of that record                  |
| `address_hints` | seq of `AddressHint`  | Where the node may be reached. May be empty.    |

`AddressHint` is a postcard enum (varint variant index, then the fields):

| Variant | Index | Fields                                        |
|---------|-------|-----------------------------------------------|
| `Ipv4`  | 0     | `ip: [u8; 4]` (raw), `port: u16` (varint)     |
| `Ipv6`  | 1     | `ip: [u8; 16]` (raw), `port: u16` (varint)    |
| `Relay` | 2     | `url: string`                                 |

Validity rules (a reader rejects the ticket otherwise, a writer never breaks them):

- `check` matches, and nothing follows it.
- `stick_id` is a valid Ed25519 point.
- At most 8 address hints. A relay URL is 1..=255 bytes of printable ASCII
  (`0x21..=0x7E`).
- The whole public part is at most 1024 bytes.

### Secret part

| Offset | Length | Field       | Value                                                         |
|--------|--------|-------------|---------------------------------------------------------------|
| 0      | 32     | `stick_key` | The StickKey                                                  |
| 32     | 4      | `check`     | First 4 bytes of `BLAKE3(public_part \|\| stick_key)`          |

`public_part` is the full public byte string, its own `check` included. So the
secret `check` also catches a secret pasted next to the wrong public part.

A reader reports which part is damaged: a ticket with a bad public `check` is
"damaged or incomplete", one with a missing or bad secret is "missing its key
(was the whole link copied?)".

## Signed manifest record (version 1)

Fixed length, 137 bytes, no postcard:

| Offset | Length | Field           | Value                                        |
|--------|--------|-----------------|----------------------------------------------|
| 0      | 1      | `version`       | `0x01`                                       |
| 1      | 32     | `stick_id`      | Ed25519 verifying key                        |
| 33     | 8      | `seq`           | u64, big-endian                              |
| 41     | 32     | `manifest_hash` | BLAKE3 hash of the manifest blob             |
| 73     | 64     | `signature`     | Ed25519 signature over the message below     |

```
message = "OurStick manifest record v1" (27 ASCII bytes) || 0x00 || record[0..73]
```

A reader accepts a record only if, in this order:

1. `version` is `0x01` and the record is exactly 137 bytes.
2. `stick_id` equals the StickId in the ticket.
3. The signature verifies under `stick_id` with strict verification
   (RFC 8032 checks, no small-order keys, canonical `S`).
4. `seq` is not smaller than `manifest_seq` in the ticket. If it is equal,
   `manifest_hash` equals the one in the ticket.
5. `seq` is strictly greater than the highest `seq` it has accepted before for
   this stick, if any. That value is kept in the Store.

   One exception, so an interrupted download can resume: a record with the
   *same* `seq` and the *same* `manifest_hash` as the one already accepted is
   not an update and is not stored again, but the reader may keep downloading
   that manifest. The same `seq` with a different `manifest_hash` is rejected.

Rule 4 gives every reader a floor from its first contact on: nobody can serve
it a manifest older than the one its ticket was made with. The ticket's
`manifest_seq` and `manifest_hash` are therefore both a hint (the reader may
start fetching that manifest at once) and a floor.

The owner increases `seq` with every publish and never reuses one. Why this
exists and what it protects against: `docs/decisions/0004-signed-manifest-records.md`.

### Test vectors

`docs/test-vectors/ticket-v1.json` has tickets and signed records built from
fixed keys, and for each record the expected outcome of checking it against the
tickets' `manifest_seq` and `manifest_hash` (rule 4). They were generated by
`docs/test-vectors/generate_ticket_v1.py`, an independent Python
implementation of the two sections above.
`crates/ourstick-core/tests/ticket_vectors.rs` checks the Rust implementation.

## Reader report (version 1)

A reader tells the owner how far it is (ADR 0015). Only someone with the
link can make or read one, and each reader speaks only for itself.

**Reader key.** When the app receives a stick it makes an Ed25519 key for
that stick alone, kept in the secret store and deleted when the stick is
forgotten. Its 32-byte public key is the **reader id**. A key per received
stick, never per device: two owners cannot match their readers.

**Body** (postcard), `ReaderReport`:

| Field      | Type                | Meaning                                                        |
|------------|---------------------|----------------------------------------------------------------|
| `counter`  | u64                 | Grows with every report of this reader, across restarts        |
| `seq`      | u64                 | The version (record `seq`) it is about                         |
| `state`    | enum                | `Downloading { percent: u8 }` (0–99), `UpToDate`, `Waiting(reason)` with `SenderAway`, `NoRoom`, `FolderMissing` or `Other`, `Paused`, `Forgotten` |
| `everything` | bool              | It gets the whole stick (nothing left out)                     |
| `kind`     | enum                | `MacOs`, `Windows`, `Linux`, `Other`                           |
| `name`     | option<string>      | Only one the receiver typed: 1–40 characters, not blank, none Cc, Cf, Zl or Zp |

No bytes, file counts or paths: the percent is of what the reader gets,
so the owner learns *that* something is left out, never what or how much.

**Signature.** Ed25519 with the reader key over
`"OurStick 2026-10-04 report signature" ‖ version (1) ‖ StickId (32) ‖ body`,
so a body signed for one version means nothing in another.

**Sealed form**, at most 480 bytes:

| Offset | Length | Field     | Value                                                    |
|--------|--------|-----------|----------------------------------------------------------|
| 0      | 1      | `version` | `0x01`                                                   |
| 1      | 24     | `nonce`   | Random                                                   |
| 25     | rest   | sealed    | XChaCha20-Poly1305 of `reader id (32) ‖ signature (64) ‖ body`, associated data `version ‖ StickId` |

The key is BLAKE3 `derive_key` with context
`OurStick 2026-10-04 report XChaCha20-Poly1305 key` over the stick key.

An owner refuses a report that is too long, cut short, of another version,
does not open (another key, another stick, a changed byte), is not signed by
the reader key it names, has bytes after the body, or breaks the rules of
the body; it logs neither its bytes nor any key. A report says only what its
reader claims: an owner shows it as that, never as a proof.

## Transport (iroh)

Peers are iroh 1.x endpoints; a NodeId is the endpoint's Ed25519 public key.
An owner serves three protocols on one endpoint:

| ALPN                | Purpose                                                    |
|---------------------|------------------------------------------------------------|
| `ourstick/record/1` | The latest signed manifest record of a stick (below)       |
| `ourstick/report/1` | A reader's sealed report of how far it is (below, ADR 0015) |
| iroh-blobs ALPN     | Blobs by BLAKE3 hash (verified streaming, range requests)  |

### `ourstick/record/1`

One request per connection, on one bidirectional stream.

| Direction | Bytes | Meaning                                                              |
|-----------|-------|----------------------------------------------------------------------|
| Request   | 32    | The StickId                                                          |
| Response  | 1     | Status: `0` = record follows, `1` = unknown stick, `2` = no record yet |
| Response  | 137   | Only for status 0: the signed manifest record (layout above)         |

The record is not trusted because of where it came from: the reader always
checks it (rules above).

### `ourstick/report/1`

One report per connection (a QUIC connection speaks one protocol, so it is
not the connection a download uses), on one bidirectional stream.

| Direction | Bytes    | Meaning                                                     |
|-----------|----------|-------------------------------------------------------------|
| Request   | 32       | The StickId, in clear, so the owner knows which key opens it |
| Request   | ≤ 480    | The sealed report (above)                                   |
| Response  | 0        | Nothing: the stream ends once the owner has taken the report; it is reset with code 1 if the report was not taken, 2 if the request was longer than 512 bytes |

The owner reads at most 512 bytes; a longer request is never read whole nor
handed on. A request for another stick, or without a report, is not taken.
At most 32 reports wait for the owner to open them (also before it starts
opening them); more are not taken. A
reader keeps a report that was not taken and sends it again later. An owner
closes a connection 10 seconds after its answer if the peer has not. The
transport opens nothing: the owner's side opens each report with the stick's
key (`SecretName::StickKey`), and drops one that does not open, saying
nothing to the reader either way.

### Relay policy (reader)

ADR 0006, version 2. Relays introduce peers and carry the record and manifest.
File data goes over whatever path exists, at once: the reader never stops and
never asks. Data that arrives through a relay is limited to a configured rate,
10 Mbps by default, and the reader says so while it lasts; data over a direct
path is never limited, and iroh keeps looking for one. Relay connections go
through an HTTP(S) proxy if `HTTP_PROXY`/`HTTPS_PROXY` (or the lowercase forms)
is set. Owners follow no policy of their own: they send over whatever path the
connection uses.

A connection can stay on the relay although a direct path exists (seen with
iroh 1.2 after a reader changed networks, and after a pause in the data on one
LAN). So a download whose data has come through the relay for a minute
connects again, which looks for a direct path from scratch, and goes on where
it was; if it is still on the relay, the next look waits twice as long, up to
30 minutes, and a direct path resets the wait.

### What an owner serves

A reader may have several gets in flight on one connection, each on its own
stream (ADR 0016: up to 32 blobs of small files at once); each is decided
on its own, as below.

An owner answers a blob request only if it is a plain get of a raw blob whose
hash is the current manifest blob or a file blob of the current manifest. It
refuses everything else: other hashes, hash sequences, get-many, observe, and
push. See ADR 0005. A file blob of the current manifest that is not ready (no
plaintext hash or outboard yet, or one that failed its check) is answered
"not ready" (`ERR_LIMIT`, ADR 0017) instead of being refused or failing.
The owner reads each request itself and decides, under one lock, what it
gets; the blob's bytes are then made from the file as above.

### Reader paths

The owner watches the paths of every reader's blob connection. If one
reader stays on the relay for 30 seconds and no reader has ever reached the
owner directly since sharing started, the owner reports that readers come
only through the relay (`ReaderActivity::only_relayed`). Direct paths need
the owner to receive what readers send, so this most likely means a
firewall drops it (on Windows, "Cancel" on the Windows Firewall question
can; with hole punching it often does not, since the owner sends first). The CLI prints a line once, and the app shows a message
that stays, both saying how to allow OurStick through the firewall of that
system. A single direct reader clears it. It is a hint, not a proof: two
networks that cannot reach each other directly (both behind strict NATs)
look the same.

## Key storage (local, not part of the wire format)

Secrets never go into SQLite. They are: the stick signing seed, the StickKey,
the stick's iroh node secret key (ADR 0009: each stick made since has its own;
older sticks use the device's), and on a reader the tickets it holds. SQLite keeps
only non-secret state (StickId, NodeId, `seq`, highest accepted `seq`, folders)
and which secret store the device uses. The name of each secret is derived
from the StickId (below), so SQLite needs no column for it. See
`docs/schema.md`.

| Platform                  | Where secrets live                                            | Status  |
|---------------------------|---------------------------------------------------------------|---------|
| Linux desktop             | Secret Service (GNOME Keyring, KWallet) over D-Bus            | v1      |
| macOS                     | The login keychain (the "legacy" keychain; the data-protection keychain needs entitlements an unsigned app lacks). Not usable from an SSH session: "User interaction is not allowed" | v1 |
| Windows                   | The user's Credential Manager, generic credentials kept on this computer only (`CRED_PERSIST_LOCAL_MACHINE`, never roaming with a domain profile, which would put the node key on a second computer). Not usable from an SSH session with key login: "ERROR_NO_SUCH_LOGON_SESSION" | v1 |
| CLI without Secret Service (macOS for now, SSH sessions, servers) | `--key-file <path>`: file encrypted with a passphrase (Argon2id + XChaCha20-Poly1305), mode 0600, chosen explicitly by the user on the device's first `share` and recorded in `device`; later runs open it and ask for the passphrase on the terminal (never from arguments or the environment). A device never switches between the two, since that would change its node key | v1 |
| Android                   | A Keystore-held AES key (hardware-backed where available) wraps the secrets, stored in app-private storage | Later (Tauri) |
| iOS                       | Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (not synced to iCloud) | Later (Tauri) |

Secret Service items use service `ourstick` and the secret's name as user:
`node-key` (the device's, only on a device set up before ADR 0009),
`stick/<hex StickId>/signing-key`, `stick/<hex StickId>/stick-key`,
`stick/<hex StickId>/node-key`, `stick/<hex StickId>/ticket`,
`stick/<hex StickId>/reader-key` (a reader's own key for a stick it
receives, ADR 0015: made at its first round, deleted with its ticket when the
stick is forgotten). A stick's node
key must give the node id the database records for it (`owned_sticks.node_id`,
or `device.node_id` for an older stick); otherwise it is not used.

### Who may share what at once (ADR 0009)

The operating system's file locks in the data folder, tried and never waited
for, released when a program ends:

- `share-<hex StickId>.lock`, per stick: one program shares a stick, or
  changes it.
- `node-key.lock`: the device's node key, which every older stick shares, so
  one older stick is shared at a time; the refusal names the one shared.
- `key-file.lock`: a key file is read whole when opened and written whole on
  every change, so a program using one holds this from before it opens it
  until it ends. With a key file, one program at a time.
- `setup.lock`, briefly, while the device or a stick is made: two programs
  never make the device twice, or two sticks for one folder.

So the app and the CLI, or two CLIs, share different sticks at the same time,
each with its own node id, when the keys are in the system keyring. Separate
node ids keep a node id alone from linking two sticks; they do not hide that
two tickets come from one computer (same public addresses and home relay in
their hints).

### Key file (version 1)

| Offset | Length | Field         | Value                                              |
|--------|--------|---------------|----------------------------------------------------|
| 0      | 4      | `magic`       | ASCII `OSKF`                                       |
| 4      | 1      | `version`     | `0x01`                                             |
| 5      | 4      | `memory_kib`  | Argon2id memory in KiB, u32 BE (8 × lanes ..= 1 GiB) |
| 9      | 4      | `iterations`  | Argon2id passes, u32 BE (1..=16)                   |
| 13     | 4      | `parallelism` | Argon2id lanes, u32 BE (1..=16)                    |
| 17     | 16     | `salt`        | Random, fixed for the life of the file             |
| 33     | 24     | `nonce`       | Random, new on every write                         |
| 57     | rest   | `ciphertext`  | XChaCha20-Poly1305 of `postcard(map<string, bytes>)`, tag included |

The key is `Argon2id(passphrase, salt)` (version 0x13, 32-byte output) with
the parameters above; defaults are RFC 9106's second recommendation (64 MiB,
3 passes, 4 lanes). The AAD is bytes `0..57`. The map goes from secret names
(as above) to secret bytes.

The file has mode 0600 and is refused if group or others can access it. It
is rewritten atomically (private temporary file, fsync, rename, fsync of the
directory) and never overwritten by "create". Only one process may use a key
file at a time; there is no locking yet.

Rules on every platform:

- If no secure store is available, fail with the reason and what the user can
  do. Never fall back to a plaintext file, silently or otherwise.
- At every start, write a test entry to the secret store, read it back and
  delete it. If that fails, stop with an error: a store that accepts writes but
  does not keep them would lose keys that tickets depend on.
- Keys are never regenerated once a ticket has been shared (CLAUDE.md rule 9).

## Open questions

- A reader telling the owner how much it still needs, so the owner can show
  "X GB remaining" (the owner only sees what it sends). Not in stage 1.

- Readers learn of new versions by asking (`get --follow`: the signed record,
  137 bytes, every 15 s over one connection). A push from the owner would be
  quicker and cheaper with many readers; decide when there are apps that stay
  open, not before.
- The owner's device going to sleep stops sharing (a MacBook sleeps after a
  minute idle by default). `share` could keep the system awake while it serves
  (macOS power assertion, systemd-inhibit on Linux, `SetThreadExecutionState`
  on Windows); decide with the desktop app.
- iOS keeps Keychain items after the app is uninstalled. On first start with an
  empty database the app finds old keys. Deleting them automatically would kill
  every ticket shared with them, so it must not happen silently. Options: offer
  to restore those sticks, or ask the user. Decided with the iOS app.
- Zeroizing secrets in memory (`StickKey`, signing seeds, the node key, the
  key-file key, decrypted manifests) when they are dropped. Today they stay in
  memory until reused, so they can end up in swap or core dumps. Would add
  `zeroize` as a direct dependency (it is already in the tree through
  `ed25519-dalek` and `chacha20poly1305`).
- A reader will need a stable node identity later (e.g. for seeds, per-reader
  access, or being reachable for updates). In v1 `get` uses a fresh random node
  key per run, because a reader needs no lasting identity and must not share the
  device's node key with a `share` running at the same time. A stable reader
  identity needs its own key in the secret store, separate from the owner's.
