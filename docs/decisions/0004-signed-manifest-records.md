# ADR 0004: Signed manifest records and the StickId

**Status:** accepted — 2026-09-28

## Context
Readers decrypt with the `StickKey`, a symmetric key that every reader holds.
Symmetric encryption proves only that the author held the key, so any reader
can produce a manifest that decrypts and looks valid. The manifest hash in the
ticket pins the first manifest, but the stick changes over time (the watcher
publishes new manifests), and later seeds will serve sticks on the owner's behalf.
Readers need to know that a new manifest comes from the owner, and that it is
newer than the one they have.

The iroh `NodeId` authenticates a machine, not a stick. Pinning it would tie the
stick to one machine and rule out seeds.

## Decision
- Every stick has an Ed25519 key pair of its own. The verifying key is the
  **StickId**, carried in the public part of the ticket. The signing key stays
  with the owner and never leaves it. It is separate from the iroh node key.
- The owner publishes a manifest as a **signed record**
  `(StickId, seq, manifest_hash)`, where `seq` is a u64 that strictly increases
  with every publish. The signature covers all three, under a domain-separation
  prefix (`docs/protocol.md` has the byte layout).
- The ticket carries the `seq` and `manifest_hash` of the latest record at the
  time it was made. They are a **hint** for the first download and a **floor**:
  the authority is always the latest signed record, but never one older than the
  ticket.
- A reader accepts a record only if the StickId matches the ticket, the
  signature verifies (`verify_strict`), `seq` is not smaller than the ticket's
  `seq` (and, if equal, names the same manifest as the ticket), and `seq` is
  **strictly greater** than the highest `seq` it has already accepted for this
  stick. So nobody (a seed, a relay, another reader) can roll a reader back to an
  older manifest, not even on first contact. That highest `seq` is stored in the
  Store (step 4).

## Why
- Only the owner can sign, so readers cannot forge updates for each other.
- Any peer can relay a signed record, so seeds need no special trust.
  The protocol still does not distinguish a Seed from any other peer.
- Putting the StickId in the ticket now means tickets shared today keep
  working when seeds and signed updates arrive.

## Consequences
- **The signing key and `seq` are long-lived state.** Losing the signing key
  means the stick can never be updated again; the owner must create a new stick
  and share new tickets. `seq` must never repeat or go back, even after a restart
  or a restore from backup, or readers will reject every new manifest.
  Step 4 persists both.
- Records are not encrypted. Anyone who sees one learns the StickId, how often
  the stick changes, and the hash of the encrypted manifest, but nothing about
  its content.
- The floor stops rollback, not withholding: a peer that answers for the owner
  can still hide newer records and keep a reader at the ticket's manifest, or at
  the last one it accepted. Only reaching the owner or an honest peer helps.
- The owner must sign a record before making a ticket, so the ticket can name
  its `seq`. A ticket made from an unsigned manifest would carry no valid floor.

## Alternatives considered
- **Trust the owner's NodeId** for updates. No new key, but ties the stick to
  one machine and makes seeds impossible without re-issuing tickets.
- **MAC with a key derived from the StickKey.** Every reader could forge.
- **No updates** (every change is a new ticket). Contradicts the v1 watcher.
