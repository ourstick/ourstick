# 0016: Fetching several blobs at once

Status: Proposed (2026-10-04)

## Context

ADR 0014 made placing and recording many small files fast, and left
fetching one blob at a time (its rule 5), to be looked at "if, after this,
it is what is slow". It is. Measured there, 10 000 files of 1–8 KB:

| Where | After ADR 0014 |
|---|---|
| Linux, sender on the same machine | 19.3 s |
| Mac, sender on the same Mac | 53 s |
| Mac, from Linux (round trip about 80 ms) | 287 s |

From Linux to the Mac each file costs about 28 ms, almost all of it the
round trip of its request: the reader asks for a blob, waits for all of
it, then asks for the next. Across the internet (100–200 ms) the same
stick would take 15–30 minutes, for 50 MB. `Rescued_Disk`
holds 146 297 files, most of them small: at 28 ms each that is over an
hour of waiting on round trips alone, whatever the bandwidth.

What is there already:

- The owner serves every request on its own QUIC stream, at once
  (`BlobsHandler` spawns a task per stream), each decided under one lock
  (ADR 0007). Nothing on the owner's side limits a reader to one request
  at a time.
- iroh-blobs' `fetch` on the reader opens one stream per blob on the same
  connection; several at once share the connection's congestion control.
- Blobs are fetched into the current batch's store (ADR 0007), and a
  batch is dropped only once all its files are placed and recorded.
- Files are placed up to 16 at once, small ones only (`PlacingQueue`,
  ADR 0014): `PLACING_AT_ONCE = 16`, at most 4 MB a file, 64 MB in all.
- Everything a fetch tells (`FileStarted`, `Progress`), the relay limit
  (ADR 0006), the path check that may connect again (`LookAgain`), and
  "not ready" (ADR 0007) assume one fetch at a time.

## Decision

1. **Small files are fetched several at once, within a batch.** Up to
   **8 blobs** in flight, each at most 4 MB (the placing queue's small
   file), at most 32 MB in all. A file bigger than 4 MB is fetched alone,
   as now: it fills the connection by itself, and its progress is told as
   it comes. The number is where a measurement stops gaining (step 1 of
   the order of work measures 1, 4, 8, 16 and 32 from Linux to the Mac,
   and takes the smallest that gets within 10% of the best).
2. **Order stays the plan's where it matters.** Fetches start in the
   plan's order; they may end in any order. A fetched file goes into the
   placing queue as it arrives; records stay in groups after their files
   are in place (ADR 0014, rule 1, unchanged). A blob is never fetched
   twice at once: a twin waits for its source, as in the placing queue.
   A batch is dropped only once every fetch of it has ended and every
   file of it is placed and recorded (unchanged).
3. **What a fetched-at-once file tells.** It tells `FileReceived` when its
   blob is here, and nothing before: no `FileStarted`, no `Progress`
   (several interleaved would make the window's bar and the CLI's line
   jump between files). A small file is at most 4 MB, so the bar still
   moves file by file. A big file tells all three, as now.
4. **One connection decides for all.** The relay limit (ADR 0006) counts
   every byte that arrives, whichever fetch it belongs to, and slows all
   of them together. The path check counts them too; when it wants to look
   for a direct path again (`LookAgain`), every fetch in flight stops,
   what arrived stays in the batch's store, and the next connection
   resumes them. A cancel stops all of them the same way.
5. **"Not ready" holds back one file, not the others.** A blob the owner
   is still preparing (ADR 0007) waits with the same backoff as now, while
   the other fetches go on; the round ends only when every file is done or
   one has waited too long (unchanged). The owner learns of several blobs
   at once (its demand), which is what its suitcases are prepared from:
   it can prepare ahead of the reader instead of one behind it.
6. **Nothing changes on the wire or on the owner's side.** Each blob is
   still one plain get on its own stream; the owner decides each as now
   (ADR 0007). The protocol document says a reader may have several in
   flight.

## In the code

- `download/fetch_queue.rs` (new, beside `placing_queue.rs`): the blobs in
  flight (at most `FETCHING_AT_ONCE = 8`, `FETCHED_FILE_BYTES = 4 MB`,
  `FETCHING_BYTES = 32 MB`), polled together on the round's task
  (`FuturesUnordered` over futures that borrow the connection; no spawned
  task, so nothing outlives the round), with the hashes in flight for
  twins. Its fetches pass a progress closure that only adds the bytes to a
  shared count and reads a stop flag; the round reads the count between
  completions for the relay limit and the path check, and sets the flag
  to stop them all.
- `download/fetch_files.rs`: a small file goes to the fetch queue; a
  completed fetch is exported and goes to the placing queue; a big file
  waits for the fetch queue to empty, then is fetched alone through
  `receive_file` as now. Its size is near 400 lines: the per-file work
  moves out with the queue.
- `download/entry.rs`: the export and the "not ready" backoff are shared
  by both paths.
- Nothing in `transport/`: `PeerConnection::fetch_blob(&self, ..)` already
  takes `&self`, and the iroh and in-memory connections allow several at
  once (the in-memory one is checked in a test).
- Tests (in memory): 8 at once at most and in the plan's order; a twin
  never fetched twice at once; a big file alone; "not ready" on one file
  while the others come; a stop and a `LookAgain` with fetches in flight
  (what arrived stays, the next round resumes); the relay limit counting
  every fetch; events: `FileReceived` for small files, all three for a big
  one. Two iroh peers: 2 000 small files in one round, all identical.
- Docs: `protocol.md` (a reader may have several gets in flight), this
  ADR's measurements.

## Consequences

- From Linux to the Mac, 10 000 small files should take a few round trips
  per 8 files instead of one per file: from 287 s to about 40 s if the
  round trip is what costs (measured in step 1 and 3, written here).
- The window's bar moves file by file for small files, without the
  in-file progress it never showed for them anyway (they arrive in
  milliseconds).
- More streams at once on the owner: at most 8 per reader, each served as
  any request is; the owner's budget and suitcases (ADR 0007) are
  unchanged and decide as before.

## Considered and not chosen

- **One request for a batch's blobs** (iroh-blobs' get-many or a hash
  sequence): one round trip per batch, but the owner refuses get-many and
  hash sequences today (ADR 0005, "What an owner serves") for good
  reasons (each blob decided on its own, "not ready" per blob); changing
  that is a change of the protocol and of the owner's one decider.
- **Several connections:** more handshakes and paths to watch, nothing
  gained over streams on one.
- **Fetching big files at once too:** a big file fills the connection
  alone; at once they would only interleave and make progress harder to
  tell.

## Order of work

1. Measure first, with a throwaway branch of `fetch_files` (not kept):
   10 000 small files Linux → Mac with 1, 4, 8, 16, 32 at once; the number
   in rule 1 is set from it.
2. `fetch_queue.rs` with its tests; `fetch_files` uses it; events as rule
   3; relay limit, path check, stop and "not ready" as rules 4 and 5.
3. Two iroh peers test; measure again Linux → Mac, Linux → Linux, the
   Windows VM; the numbers in this ADR. Docs.
