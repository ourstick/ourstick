# ADR 0005: Encrypted blob cache on disk, iroh-blobs behind `Transport`

**Status:** accepted — 2026-09-28

## Context
The owner serves each file as an encrypted blob (ADR 0003). The ciphertext must
never reach a peer before `encrypt_blob` has confirmed that the file did not
change while it was encrypted (`PlaintextChanged`). Otherwise two versions of a
chunk could go out under the same nonce.

Blobs are transferred with iroh-blobs 0.103 over iroh 1.2. iroh-blobs serves
verified BLAKE3 ranges (bao) from its own store, and only from blobs that are
fully present there together with their outboard. It has no hook to produce
bytes when a request arrives.

## Decision
**Cache on disk.** The owner encrypts every file into a private temporary file,
and only after `encrypt_blob` returns Ok imports it into a per-stick iroh-blobs
`FsStore`, the cache. Only hashes of the current manifest (and the manifest blob
itself) are served. The rule "no ciphertext before the check" is therefore
structural: a blob that failed the check never enters the store, and a blob that
is not in the current manifest is never served.

- **Serving only what is allowed.** The provider runs with an intercepting
  event handler: it accepts a request only if it is a plain `Get` for a raw blob
  whose hash is in the current allowlist. Everything else is rejected: other
  hashes, hash sequences, get-many, observe, and push. Push is rejected
  explicitly.
- **Removing old blobs.** The owner holds an in-memory `TempTag` for every blob
  of the current manifest and for every import in progress, from the moment of
  import. When the manifest changes, the tags of blobs no longer in it are
  dropped and the store's periodic GC deletes them. We do not rely on named tags
  because of upstream issue #247 (a race in GC can delete a blob between
  `tags.set` and dropping its temp tag). Until the first scan after start has
  re-acquired the temp tags, the GC's protect callback aborts every run, so a
  restart never wipes the cache.
- **Reader.** The reader keeps a per-stick `FsStore` so interrupted downloads
  resume (only missing ranges are requested). It runs no GC, since GC would
  delete partial, untagged blobs and lose the resume state. After a complete sync
  its files are already in the output folder, so the store is emptied. The
  core does not do this itself: `download()` leaves the store as it is, and the
  caller decides with `Fetcher::shutdown(keep)`. The CLI passes `keep = false`
  after a successful download (store deleted) and `keep = true` after an error
  (store kept for the resume).
- **Visibility.** `share` prints where the cache is and how much space it will
  take before it starts encrypting.

## Why not on-the-fly encryption
- iroh-blobs cannot serve bytes that are not in its store. On-the-fly would mean
  writing our own provider and bao outboard computation.
- Encrypting while sending means bytes leave before `encrypt_blob` can know
  whether the file changed. That is exactly the case ADR 0003 forbids.
- On-the-fly also re-encrypts for every reader and every resumed range.

## Costs
- **Disk space** about equal to the shared folder, plus about 0.4 % for bao
  outboards and 16 bytes per 64 KiB chunk. Shown before `share` starts.
- **First share** encrypts everything once. Because the nonce is derived from the
  content (ADR 0003), unchanged files give the same hash later; the watcher and a
  future `(path, size, mtime)` cache can skip them.

## Maturity risk: iroh-blobs 0.103
The iroh-blobs README states that the 0.9x/0.10x line is "not yet considered
production quality" and points production users at 0.35, which only works with
iroh before 1.0. We found a real issue while integrating it: the GC race in #247.

Mitigations:
- All iroh and iroh-blobs code lives behind the `Transport` trait
  (`crates/ourstick-core/src/transport/`). Nothing else in the core, the CLI or
  the app imports iroh types. Replacing iroh-blobs (with a later version, or with
  our own range protocol on plain iroh streams) touches one module.
- The wire format of our own data (blobs, manifest, records, tickets) does not
  depend on iroh-blobs: a blob hash is plain BLAKE3 of the blob bytes.
- The integration tests exercise what we rely on: full transfer, resume, a
  corrupted blob, a stale record, and a push that must be refused.

## Alternatives considered
- **On-the-fly encryption with our own provider.** Rejected, see above.
- **iroh-blobs `ImportMode::TryReference` on the plaintext file.** Impossible:
  the served bytes are ciphertext, not the file.
- **Named tags for liveness.** Rejected because of #247.
