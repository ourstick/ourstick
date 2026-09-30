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

1. Owner encrypts every file and the manifest into blobs in its cache
   (ADR 0005), signs a record for the manifest, and serves both over iroh.
2. Owner produces a ticket (link / QR).
3. Reader decodes the ticket and connects to the owner's NodeId (address hints
   first; iroh address lookup and relays otherwise).
4. Reader asks for the latest signed record (`ourstick/record/1`, below) and
   checks it against the ticket and what it has seen before.
5. Reader fetches the manifest blob by the record's hash, decrypts and
   validates it.
6. Reader fetches each file blob by hash (iroh-blobs, verified BLAKE3 streaming,
   resumable), decrypts it to a temporary file, and renames it into place.

## Manifest wire format (version 1)

```
manifest_bytes = format_version:u8 || postcard(Manifest)
```

`format_version` is `1`. A reader rejects any other value. Postcard is not
self-describing, so any change to the fields below needs a new version.

`Manifest` is a postcard sequence of `Entry` (varint length, then the entries).
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
- Entries are strictly sorted by `path`, compared as bytes, which rules out
  duplicates and makes the encoding canonical.
- Every `path` is non-empty, contains no `\` and no NUL, and splitting it on `/`
  gives no component that is empty, `.` or `..`. This rules out absolute paths
  and escaping the output folder.

The rules are platform-neutral: a manifest that follows them is valid on the
wire. Before writing anything, a reader additionally refuses the whole
manifest (nothing is written, its record is not accepted) if any path:

- has a component that is not a single plain name on the reader's platform;
- has a component that looks like a Windows drive (an ASCII letter followed
  by `:`, e.g. `c:evil`), on every platform, so a stick behaves the same
  everywhere;
- on Windows, has a component with `< > : " | ? *` or a control character,
  ending in a dot or space, or named like a device (`CON`, `PRN`, `AUX`,
  `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`, with or without an extension);
- starts with the reader's partial directory `.ourstick-partial`, in any
  letter case;
- equals another path when letter case is ignored, or is a file that another
  path needs as a folder (e.g. `a` and `a/b`, also `A` and `a/b`).

A reader also refuses to write through a symbolic link that exists in its
output folder, checking when a file starts and again right before it is moved
into place. Mapping such names to safe ones instead of refusing them is an
open question.

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
- A **drive** (the app's stick, `owned_sticks.drive_size`) carries the file
  `.ourstick-drive`, holding its StickId in hex. Without it (an ejected disk
  image, an empty mount point, another stick's drive) the owner does not
  publish, and keeps serving the last version. A drive whose files hold more
  than its size is not published either.
- A version changes only when the folder does: if the new manifest hash equals
  the one in the last signed record, that record is served again and no new
  `seq` is signed.
- A file is encrypted again only if it changed since the last publish, judged
  by a stamp (see "Known limits" below); otherwise its recorded blob hash is
  used. `share --rescan` reads and encrypts every file.
- While serving, the owner watches the folder (inotify, FSEvents, …; every
  60 s if the system cannot report changes). After a sign of change it waits
  until the folder has been quiet for 2 s and no file's size or times change
  over 1 s, then publishes as above. Opening and reading files are not changes.
  A failed publish (e.g. an unreadable file) is reported and tried again at
  the next change or after 30 s.

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
- Files that left the stick are kept and reported, never deleted.
- If the owner refuses a blob during a download, most likely because it
  published a new version and stopped serving the old blobs, the reader
  fetches the record again. With a newer version it goes on with that one
  (files already written count as already here); with the same version the
  refusal stands. At most 10 new versions are followed per download.
- A version counts as complete in a folder (`joined_sticks.completed_*`) only
  if every file of it is there; a file left as changed keeps it incomplete.

### Known limits

- **Unchanged-file detection is a heuristic.** The owner records, per file,
  the size, mtime and (on Unix) ctime in nanoseconds at the last publish
  (`owned_files`, `docs/schema.md`), and does not read a file whose stamp is
  unchanged. A change that keeps all three is not seen, and readers keep
  getting the old content for that file. On Unix, ctime changes on every write
  and cannot be set back by ordinary tools, so this needs a file system with
  coarse or missing timestamps, a clock set backwards, or a deliberate change of
  the system time. Elsewhere only size and mtime count, and a tool that
  restores mtime after writing (some sync and archive tools do) is enough.
  `ourstick share --rescan` ignores the record and reads every file.
- **A GC error in the log after sharing stops.** The owner's blob cache is an
  iroh-blobs store with garbage collection on. Its GC task outlives the store:
  after Stop sharing, a Mount (which starts sharing again), Ctrl-C or Quit it
  wakes once more, finds the store closed and logs
  `ERROR iroh_blobs::store::gc: error during gc run: Error::Io`. Nothing is
  wrong; the next store collects as usual. In iroh-blobs 0.103 any GC error
  also ends GC until the store is opened again, and GC cannot be run from
  outside the crate; an upstream issue is pending (`docs/todo.md`).

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

## Transport (iroh)

Peers are iroh 1.x endpoints; a NodeId is the endpoint's Ed25519 public key.
An owner serves two protocols on one endpoint:

| ALPN                | Purpose                                                    |
|---------------------|------------------------------------------------------------|
| `ourstick/record/1` | The latest signed manifest record of a stick (below)       |
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

### Relay policy (reader)

ADR 0006, version 2. Relays introduce peers and carry the record and manifest.
File data goes over whatever path exists, at once: the reader never stops and
never asks. Data that arrives through a relay is limited to a configured rate,
10 Mbps by default, and the reader says so while it lasts; data over a direct
path is never limited, and iroh keeps looking for one. Relay connections go
through an HTTP(S) proxy if `HTTP_PROXY`/`HTTPS_PROXY` (or the lowercase forms)
is set. Owners follow no policy of their own: they send over whatever path the
connection uses.

### What an owner serves

An owner answers a blob request only if it is a plain get of a raw blob whose
hash is the current manifest blob or a file blob of the current manifest. It
refuses everything else: other hashes, hash sequences, get-many, observe, and
push. Blobs leave the cache once they are no longer in the current manifest.
See ADR 0005.

### Reader paths

The owner watches the paths of every reader's blob connection. If one
reader stays on the relay for 30 seconds and no reader has ever reached the
owner directly since sharing started, the owner reports that readers come
only through the relay (`ReaderActivity::only_relayed`). Direct paths need
the owner to receive what readers send, so this most likely means a
firewall drops it: on Windows, "Cancel" on the Windows Firewall question
does exactly that. The CLI prints a line once, and the app shows a message
that stays, both saying how to allow OurStick through the firewall of that
system. A single direct reader clears it. It is a hint, not a proof: two
networks that cannot reach each other directly (both behind strict NATs)
look the same.

## Key storage (local, not part of the wire format)

Secrets never go into SQLite. They are: the stick signing seed, the StickKey,
the iroh node secret key, and on a reader the tickets it holds. SQLite keeps
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
`node-key`, `stick/<hex StickId>/signing-key`, `stick/<hex StickId>/stick-key`,
`stick/<hex StickId>/ticket`.

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
- Mapping names that a reader cannot write (Windows device names, `:`,
  letter-case collisions) to safe ones, instead of refusing the manifest.
- A reader will need a stable node identity later (e.g. for seeds, per-reader
  access, or being reachable for updates). In v1 `get` uses a fresh random node
  key per run, because a reader needs no lasting identity and must not share the
  device's node key with a `share` running at the same time. A stable reader
  identity needs its own key in the secret store, separate from the owner's.
