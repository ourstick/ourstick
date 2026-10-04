# 0016: Fetching several blobs at once

Status: Proposed (2026-10-04), revised after the design reviews

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
  (ADR 0007).
- iroh-blobs 0.103's `fetch` opens its own stream per blob on the
  connection and writes through the store's actor: many at once on one
  connection and one store are supported. QUIC allows 100 streams at once
  by default (iroh does not lower it) and bounds the bytes in flight per
  stream and per connection itself.
- Blobs are fetched into the current batch's store (ADR 0007); switching
  or dropping a batch shuts its store down.
- Files are placed up to 16 at once, small ones only (`PlacingQueue`,
  ADR 0014: a file of at most 4 MB is small; a bigger one is placed
  alone).
- On the reader, everything that watches a fetch lives in its progress
  callback, which assumes one fetch at a time: the per-file events
  (`FileStarted`, `Progress`), the user's stop (an event's `Break`), the
  relay limit (ADR 0006: `RelayLimiter::wait`, which takes the whole debt
  of whoever calls it), and the path check that may connect again
  (`LookAgain`).
- A fetch is stopped by closing the whole connection
  (`IrohConnection::fetch_blob`, `blobs.close(0, "cancelled")`): every
  other fetch on it then fails as "interrupted".
- On the owner, the suitcases' keeper (ADR 0007, `share/keeper/state.rs`)
  takes each ask: an unserved one puts its suitcase at the front of the
  preparation queue, and the reader's place becomes the place of its
  latest ask. With one ask at a time both are stable; with several
  interleaved across a suitcase's end they are not (below, rule 6).

## Decision

1. **Small files are fetched several at once, within a batch.** A small
   file is what the placing queue calls small (at most 4 MB, one rule for
   both queues); a bigger file is fetched alone, after the fetches in
   flight end, as now: it fills the connection by itself, and its
   progress is told as it comes. **32 small ones at once**
   (`FETCHING_AT_ONCE`), set by measuring (step 1, below: the smallest
   within 10% of the best). No byte limit of our own: QUIC already bounds what is in
   flight, and each file is at most 4 MB.
2. **Order stays the plan's where it matters.** Fetches start in the
   plan's order and may end in any order. A fetched file goes into the
   placing queue as it arrives; records stay in groups after their files
   are in place (ADR 0014, unchanged). A blob is never fetched twice at
   once: a twin waits for its source to be fetched and placed (the blob's
   file in `partial/` is named by its hash), then is copied, as today.
   Every fetch of a batch has ended before its store is switched or
   dropped.
3. **What is told, and the stop.** A file fetched with others tells
   `FileReceived` when its blob is here, and nothing before (several
   interleaved `FileStarted`/`Progress` would make the window's bar and the
   CLI's line jump between files); a big file tells all three, as now. The
   user's stop is checked at every event the round tells, so at each
   completion too. The CLI prints its "so far" line on `FileReceived` as
   on `Progress`, under the same throttle, so a stick of small files is
   not silent.
4. **One connection decides for all, in one place.** The round, not a
   fetch, owns the path check and the stop: every fetch's callback adds
   its bytes to one shared count and returns what was last decided (go on;
   pause until an instant; stop). The round reads the count every time a
   fetch ends or a short tick passes. The relay limit is one too, but asked
   as the bytes come, not only at the round's looks (a blob that arrives
   within one look would go unlimited): while the round polls the fetches
   the limiter is theirs, one of them asks it every 256 KB of all of them,
   and the pause it gives holds them all. When it stops them (the user's stop, or
   `LookAgain`), it sets the decision first, then waits for every fetch in
   flight to end and ignores their errors (closing the connection makes
   them fail as interrupted): the round's outcome comes from the decision,
   never from whichever error came first. What arrived stays in the
   batch's store; the next connection resumes it.
5. **"Not ready" holds back one file, not the others.** A blob the owner
   is still preparing (ADR 0007) waits with the same backoff as now; the
   other fetches in flight go on, and the round ends only when every file
   is done or one has waited too long (unchanged). A file waiting keeps
   its place in the queue: the reader does not run ahead into later
   suitcases (files in the plan's order share suitcases, so one not ready
   usually means its neighbours are not either), and if every place waits,
   the queue waits, as one fetch at a time waits today. The waiting ones
   are asked again on the round's tick, each at its own backoff.
6. **The owner keeps its order with several asks at once.** In the
   keeper, an ask of a connection waits until its blob is served to that
   connection, or until `IN_USE` passes without it being asked again. A
   connection's place is the lowest suitcase among its waiting asks, or,
   when none waits, the highest suitcase among its asks served (asks end
   in any order: the highest, never the latest, so the place never moves
   back). The waiting asks at the front of the preparation queue are kept
   in suitcase order, and a suitcase is held while a connection waits on
   it, is on it, or is on the one before (as before, within `IN_USE`). One
   it was served from and moved past is not held: a transfer still coming
   from it holds it itself (ADR 0007), and holding it longer would only
   keep the next one from being prepared. So asks that straddle a
   suitcase's end prepare the earlier one first and keep it held while its
   blobs are still asked for. Nothing else on the owner changes: each blob is one
   plain get on its own stream, decided as now. The protocol document
   says a reader may have several gets in flight.

## In the code

- `download/fetch_queue.rs` (new, beside `placing_queue.rs`; tested with
  in-memory peers in `tests/several_at_once.rs`): the blobs in flight (at most
  `FETCHING_AT_ONCE`, small files only by `placing_queue`'s rule), polled
  together on the round's task (`FuturesUnordered` over futures that
  borrow the connection: no spawned task, nothing outlives the round),
  with the hashes in flight for twins, the shared byte count and the
  round's decision (an atomic the callbacks read). `drain()` waits for all
  of them and drops their errors.
- `download/fetch_files.rs`: a small file goes to the fetch queue; each
  completed fetch is exported and handed to the placing queue; a big file
  waits for the fetch queue to drain, then is fetched alone through
  `receive_file` as now. Between completions (and on a tick) it runs the
  relay limit, the path check and the stop for all of them. Export, the
  `partial/` checks and the blob store stay off the async threads
  (`run_blocking`, or the store's own async calls), as now. The per-file
  steps shared by both ways (export, "not ready" backoff) move to
  `download/fetch_one.rs`, so neither file passes 400 lines.
- The relay limiter (`relay.rs`, unchanged) moves into the queue's shared
  decision while the round polls the fetches and back after; asked every
  256 KB of all of them and at each look, its pause is the "pause until"
  every fetch reads. As before, "the relay limit applies" is told at a path
  check (every 4 MB).
- `share/keeper/state.rs`: rule 6.
- Consumers of the per-file events: `cli/progress.rs` (rule 3's line on
  `FileReceived`), `apps/app/.../receiving/told.rs` (`Landed` already counts
  on `FileReceived`), `download/reporter.rs` (already treats the three
  alike); `download/entry.rs` stops telling `FileStarted` for small files
  fetched with others.
- Nothing in `transport/`: `PeerConnection::fetch_blob(&self, ..)` already
  allows several at once (the in-memory connection is checked in a test).
- Tests (in memory): at most `FETCHING_AT_ONCE` at once, started in the
  plan's order; a twin never fetched twice at once; a big file alone,
  after the small ones drain, with its `FileStarted` after their
  `FileReceived`; "not ready" on one file while the others come; a user
  stop and a `LookAgain` with fetches in flight end as the decision says,
  however the others fail, and the next round resumes; the relay limit
  pausing all fetches; the CLI line on `FileReceived`. The keeper: asks
  straddling a suitcase's end prepare the earlier one first, and keep it
  held. Two iroh peers: 2 000 small files in one round, all identical.
- Docs: `protocol.md` (a reader may have several gets in flight), ADR
  0014's rule 5 points here, `todo.md`, this ADR's measurements.

## Consequences

- From Linux to the Mac, `FETCHING_AT_ONCE` files share each round trip:
  at 80 ms, 8 at once is about 100 files a second (10 000 in about 100 s),
  32 at once about 400 (about 25 s, if nothing else limits first). Step 1
  measures it; the numbers go here.
- The window's bar moves file by file for small files, without the
  in-file progress it never showed for them anyway (they arrive in
  milliseconds); the CLI's line too.
- More streams at once on the owner: at most `FETCHING_AT_ONCE` per
  reader, each served as any request is; its budget and suitcases (ADR
  0007) decide as before, in a steadier order (rule 6).

## Considered and not chosen

- **One request for a batch's blobs** (iroh-blobs' get-many or a hash
  sequence): one round trip per batch, but the owner refuses get-many and
  hash sequences (protocol.md, "What an owner serves"), so each blob is
  decided on its own and "not ready" is per blob; changing that is a
  change of the protocol and of the owner's one decider.
- **Several connections:** more handshakes and paths to watch, nothing
  gained over streams on one.
- **Fetching big files at once too:** a big file fills the connection
  alone; at once they would only interleave and make progress harder to
  tell.
- **A byte limit of our own on what is in flight:** QUIC's flow control
  already bounds it.

## Order of work

1. Measure first, with a throwaway change of `fetch_files` (not
   committed): 10 000 small files Linux → Mac at 1, 4, 8, 16, 32 and 64 at
   once; `FETCHING_AT_ONCE` is set from it, and the numbers written here.
2. a. `fetch_queue.rs` and `fetch_one.rs` with their tests, used with one
   fetch at a time: behaviour unchanged, `cargo test` green.
   b. The keeper's rule 6, with its tests.
   c. `FETCHING_AT_ONCE` raised; events, the CLI line, the round's
   decision for the relay limit, the path check and the stop (rules 3
   and 4), "not ready" (rule 5), with their tests.
3. Two iroh peers test; measured again Linux → Mac, Linux → Linux, the
   Windows VM; the numbers here; docs; status Accepted.

## Measured

Step 1, 2026-10-04: 10 000 files of 1–8 KB, `ourstick get` on the Mac from
an `ourstick share` on Linux (direct, over the home network, round trip
about 80 ms), each run from a fresh folder, with a throwaway change that
fetched a batch's small blobs N at once before placing them as before
(not committed):

| At once | Time |
|---|---|
| 1 | 264 s |
| 4 | 121 s |
| 8 | 90 s |
| 16 | 69 s |
| 32 | 60 s |
| 64 | 56 s |

32 is within 10% of the best (64): `FETCHING_AT_ONCE = 32`. What is left
at 32 and 64, about 55 s, is placing the files on the Mac (each synced
with `F_FULLFSYNC`), as from the Mac to itself in ADR 0014 (53 s): the
network's waiting is gone.
