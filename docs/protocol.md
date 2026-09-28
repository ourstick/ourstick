# OurStick protocol

_Draft. Updated whenever the wire format or the ticket format changes._

## Concepts

- **Stick** — a named, keyed collection of root folders.
- **Manifest** — index of all files: path, size, BLAKE3 hash of the encrypted blob.
- **Ticket** — everything a reader needs: stick id, node address hints, manifest hash, decryption key.
- **Peer** — Owner, Reader or Seed. The protocol does not distinguish a Seed from any other peer.

## Flow

1. Owner builds manifest, encrypts blobs and manifest, announces via iroh.
2. Owner produces ticket → string / QR.
3. Reader decodes ticket, connects, fetches manifest by hash, verifies, decrypts.
4. Reader fetches blobs by hash (resumable), verifies each chunk, decrypts, writes to disk.

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

The rules are platform-neutral. A receiver must still map names that are
invalid on its own platform (e.g. `:` or reserved names like `CON` on Windows)
before writing to disk.

### What the manifest hash covers

The manifest hash is the BLAKE3 hash of the manifest blob as sent. All fields,
including `mtime`, are covered: a reader can trust every field it decodes.
Changing only a file's `mtime` therefore produces a new manifest, but not new
file blobs.

The manifest bytes are encrypted into a blob in the format below, like any
file (not implemented yet). Because the nonce prefix is derived from the
content (ADR 0003), the manifest hash stays the same while the manifest is
unchanged, and changes whenever any field changes.

### Owner-side rules for building a manifest

- Symlinks are not followed and not included. Sockets, devices and other
  non-regular files are skipped. Empty directories are not recorded.
- A file name that is not UTF-8, or that breaks the path rules above, is an error.

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

## Open questions

- How manifest updates are announced to connected readers.
- Encrypting the manifest into a blob (format above) and fetching it by hash.
  Decided in step 3, together with what the ticket carries.
- How the owner guarantees that no ciphertext reaches a peer before the
  `PlaintextChanged` check passes (ADR 0003). Decided in step 5, with the transport.
