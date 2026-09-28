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
| `size`  | u64 (varint)      | Size of the file in bytes                                 |
| `mtime` | u64 (varint)      | Modification time, whole seconds since the UNIX epoch (clamped to 0) |
| `hash`  | `[u8; 32]` (raw)  | BLAKE3 hash of the file's blob                            |

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

Since the manifest is encrypted with a fresh nonce, its hash changes on every
publish even when the content is unchanged. Peers detect "nothing changed" by
comparing decoded manifests, not hashes.

### Owner-side rules for building a manifest

- Symlinks are not followed and not included. Sockets, devices and other
  non-regular files are skipped. Empty directories are not recorded.
- A file name that is not UTF-8, or that breaks the path rules above, is an error.

## Open questions

- Chunk size and encryption granularity (per file vs per chunk).
- How manifest updates are announced to connected readers.
- Encryption of the manifest and blobs (step 2): until then `hash` is over the
  plaintext, which is what the blob looks like with no cipher.
