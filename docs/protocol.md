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

## Open questions

- Chunk size and encryption granularity (per file vs per chunk).
- How manifest updates are announced to connected readers.
