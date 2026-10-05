# ADR 0003: Deterministic nonce prefix per blob

**Status:** accepted — 2026-09-28

## Context
Blobs are encrypted with XChaCha20-Poly1305 in 64 KiB chunks. Each chunk's nonce
is `prefix (19 bytes) || chunk index (u32 BE) || last flag (1 byte)`, so the only
per-blob choice is the prefix. The manifest records the BLAKE3 hash of each
encrypted blob, and readers fetch blobs by that hash.

## Decision
The nonce prefix is derived from the content, not drawn at random:

```
nonce_key = BLAKE3-derive_key("OurStick 2026-09-28 blob nonce prefix key", stick_key)
prefix    = BLAKE3-keyed(nonce_key, BLAKE3(plaintext))[0..19]
```

`nonce_key` is derived from the stick key with its own context, so the stick key
is never used directly as both an AEAD key and a hashing key. The same nonce
prefix only ever comes from the same plaintext, which gives the same ciphertext,
so a repeated nonce never encrypts two different plaintexts.

The prefix is stored in the blob header, which is covered by the blob hash and is
the AAD of every chunk.

## Why
A random prefix would change the ciphertext, and so the hash, on every scan:

- Every watcher event would republish every file, and readers would re-download
  everything, even when nothing changed.
- To serve the same bytes it announced, the owner would have to store either the
  nonces (per file, in SQLite) or the ciphertexts themselves.

With a derived prefix, the same folder under the same key always gives the same
hashes. The owner needs no state to reproduce a blob and can encrypt on the fly.
The manifest, encrypted the same way, also keeps a stable hash while its content
is unchanged.

## Consequences
- **Equality leaks.** Anyone who sees the blobs, even without the key, can tell
  that two files in the same stick have identical content, because their blobs
  are identical. They learn nothing else about the content. Across sticks
  (different keys) nothing is revealed. Changing the key changes every blob.
- **Two passes per file.** The plaintext is hashed before it is encrypted. A
  later cache in the Store keyed by `(path, size, mtime)` can skip the first pass.
  *(ADR 0017, 2026-10-05: suitcases are gone. The owner's store makes each
  blob from its file when asked and checks every 16 KiB group against the
  catalogue's outboard before sending it; the guarantee is the same.)*
  *(ADR 0007, 2026-09-30: the owner's suitcases do so. They take the prefix
  from the plaintext hash the catalogue recorded for that very blob and read
  the file once, checking the hash as they go; if the file changed, what was
  written is deleted at once and never served, because a blob is served only
  if its hash is the manifest's. `encrypt_blob_hashed_before`.)*
- **Changing files.** If a file changes between the two passes, its prefix no
  longer belongs to the bytes being encrypted. `encrypt_blob` hashes the input
  again during encryption and fails with `PlaintextChanged`. Ciphertext must
  never reach a peer before that check passes; otherwise two versions of a chunk
  could go out under the same nonce. How the transport guarantees this (for
  example, serving from a verified immutable copy instead of encrypting a live
  file while sending it) is decided in step 5, with the transport.
- `NoCipher` has no prefix and gets none of this; it offers no confidentiality.

## Alternatives considered
- **Random prefix stored in SQLite.** Keeps the equality private, but costs state,
  rescans republish everything, and a lost database means re-sending everything.
- **Random prefix, ciphertext kept on disk.** Doubles disk use for every stick.
- **Nonce derived per chunk from that chunk's content** (SIV per chunk). Safe even
  for files that change while being read, but needs a 24-byte nonce stored per
  chunk and leaks equality per chunk, not just per file.
