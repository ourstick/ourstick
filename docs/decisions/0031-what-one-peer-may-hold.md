# 0031. What one peer may hold

Date: 2026-11-04. Status: accepted (found by the security review before the
repository went public; Giorgos asked for every finding to be fixed).

## Context

An owner serves its sticks to whoever connects. A node id is in every link's
public part, and a new node id costs nothing, so the number of peers, of
their connections and of their streams is the peer's choice. A reader with
the link may also ask for every blob it likes, as often as it likes.

The review found what that allowed (a test, `tests/serving_limits.rs`,
reproduces the first):

- **Gets that are never read.** Each get keeps a file and its hash tree
  open (ADR 0017) until it is sent. A reader that asks and never reads,
  on 100 streams of each of a few connections, kept hundreds of files open.
  macOS gives an app 256: the app could then open no file, no database,
  no socket, for any stick. A file kept open also keeps its disk from
  being ejected.
- **Requests that never end.** The request of a get was read with
  iroh-blobs' own limit of 1 MiB, with no time limit, on each of up to
  100 streams per connection: about 100 MiB per connection, without even
  knowing a link.
- **Record and report protocols** waited for a stream and its request with
  no time limit either.

## Decision

1. **Requests are small and come at once.** A request is read by the owner
   itself, at most 64 KiB (a get is under 100 bytes; a resume adds a few
   per gap), within 10 seconds of its stream. On the record and report
   protocols the stream comes within 10 seconds of the connection.
2. **QUIC holds little for a peer.** An owner's endpoint takes at most 33
   streams at once per connection (32 gets, ADR 0016, and one more), no
   unidirectional streams, 256 KiB unread per stream and 1 MiB per
   connection. A reader's endpoint keeps iroh's defaults: it downloads.
3. **Transfers are counted.** At most 32 at once per connection: its next
   stream is taken only once one ends, so the peer waits. At most 64 at
   once in the whole program, all sticks together, since the limit is the
   program's open files: one more is answered "not ready" (`ERR_LIMIT`),
   and the reader asks again later, as it does for a file not ready yet.
   Only a file's transfer takes one of the 64: the manifest is served from
   memory, and anyone with a link's public part may ask for it. At most 128
   connections at once per stick on the blobs protocol; one more closes the
   one idle the longest (with no transfer), so that connections asking
   nothing, which a node id is enough to open, never keep a reader out.
4. **A stalled transfer ends.** If the reader takes nothing for 2 minutes
   (each write to the stream waits at most that long), or less than 64 KiB
   in 2 minutes (a trickle that keeps each write going), the transfer ends
   and its file is closed. The record protocol's answer is given within 10
   seconds too, and an owner keeps no datagrams. A reader that was asleep (a laptop closed, a
   phone in Doze) asks again and resumes from what it has.
5. **A reader takes no more than it was told.** The size of a manifest
   entry is the owner's word, and a blob's hash covers whatever the owner
   sends. A reader fetches at most the blob length the entry's size allows
   (header, size and one tag per chunk, for any cipher), and stops as soon
   as more shows; the manifest blob keeps its own limit. Together with the
   manifest's bounds on paths and sizes (protocol.md, "Validity rules"),
   the space a reader asks the user for is the space it uses.

6. **Asks are a hint, so many a second.** Each get the owner does not
   refuse tells its keeper what was asked (ADR 0017), on a queue with no
   bound. A stick's "not ready" asks, the ones the keeper acts on, are
   told at most 64 a second, after a burst of 256; the rest are answered
   as ever, only not told. Served ones are bounded by the transfers. A reader fetching
   32 files at once, asking again after a "not ready" a quarter of a second
   later and then less often, stays well under it.

## Consequences

- An owner's open files from serving are bounded: two per transfer, 64
  transfers. The test measures fewer than 170 more files open for 132
  stalled gets, against 385 without the limits.
- Someone with the link can still take the 64 places, from a few
  connections, and keep them by reading slowly (at least 64 KiB each 2
  minutes): other readers are then told "not ready" until they free. The places are
  the program's, so that stops the file transfers of every stick the
  computer shares, not only that stick's (Amendment 1); the remedy is the
  one the protocol has for any unwanted reader, a new link for the stick
  whose link was given.
- Three readers each fetching 32 small files at once share 64 places: the
  third waits a little. Small files are sent quickly, so this is rarely
  seen.
- The 64 places are one static for the program, like the made-blobs
  runtime (ADR 0017, rule 8): both are limits of the process, not of a
  stick.

## Amendment 1: only a file's transfer counts (2026-11-05)

Found by a second security review. Rule 3 counted a connection as having a
transfer as soon as it opened a stream, before its request was read. Anyone
with a node id could open the 128 connections, each with a stream that asks
nothing (a new one every 10 seconds, before the old one ends), and every
new connection of a real reader was closed as "busy".

- A connection counts as busy only while it is sending a file: a stream
  with no request yet, a request refused or "not ready", and the manifest
  (which anyone with a link's public part may fetch) never keep it from
  being closed for a new one. The test opens 128 connections with a silent
  stream each, and a reader still gets its file.
- The 64 places of rule 3 are the program's, for all sticks together, as
  the last consequence says: someone holding them with one stick's link
  holds back the file transfers of the computer's other sticks too. Kept
  so, since the limit is the program's open files; a share per stick is
  left for when it is seen to matter.
