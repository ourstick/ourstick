# ADR 0017: Blobs made when asked, from the files themselves

**Status:** accepted — 2026-10-05; implemented 2026-10-05 (steps 1–5).
Supersedes ADR 0007's suitcases for
the owner (its catalogue, partial versions, readers' batches and "not
ready" stay), its Amendment 2 with them, and the owner's part of the space
ledger. The reader is unchanged; the wire protocol is unchanged. Reviewed
by two other models against our code and iroh-blobs 0.103 / bao-tree
0.16.1; this version takes in their changes.

## Context

ADR 0007 keeps, for every blob a reader asks for, an encrypted copy in a
"suitcase": an iroh-blobs `FsStore` of up to 4 096 blobs (1 GB), prepared by
reading and encrypting its files, evicted whole when the budget needs room.
Measured on 2026-10-04/05 (owner of 100 000 small files, one local reader):

- Each `FsStore` runs its own tokio runtime and redb database. Suitcases
  stay open until the budget needs their room, so threads grow with them
  (macOS 40 → 208, Windows 33 → 82), and so does memory: macOS 139 → 206 MB,
  Linux 243 → 377 MB. On Linux glibc's `malloc_info` showed why: 87
  arenas, 333 MB taken from the system, 246 MB of it free but kept, 87 MB in
  use.
- Evicting idle suitcases (Amendment 2, a test build) kept macOS flat
  (110 → 121 MB) but not Linux without an arena limit (238 → 306 MB):
  every store that comes and goes brings its own threads.
- A suitcase also costs a wait ("not ready") the first time it is asked
  for, disk space up to the budget (2–40 GB), and a whole re-read when it is
  asked for again after it left.

What makes suitcases necessary is iroh-blobs: it keeps blobs only in its
own stores and cannot delete one blob (ADR 0007). But it can serve from a
store of ours: `MemStore::from_sender` (iroh-blobs 0.103 `store/mem.rs:113`,
public) wraps any actor that answers its commands (`irpc::Client` of the
public `api::proto::Request`), and `handle_get`, which our handler already
calls with a store of its choice, needs only one of them for a raw blob at
offset 0, `ExportBao` (`provider.rs:427, 604`; hash sequences, which use
others, are refused by `Serving` before). That actor can make each blob's
bytes when they are asked for.

## Decision

Each stick the owner shares gets one store actor of ours, on the program's
runtime, that makes a file's encrypted bytes from the file as they are
sent, each 16 KiB group checked against a tree of hashes made by the
catalogue before it is sent. No encrypted copy is kept.

1. **The catalogue keeps an outboard.** When the pass reads a file (it
   already encrypts it in full to hash the blob, ADR 0007), it also builds
   the blob's bao outboard (pre-order, iroh's 16 KiB chunk groups) and writes
   it to the stick's cache folder, `outboards/<blob hash>.obao`, through a
   temporary file and a rename. A blob of at most 16 KiB has none: it is a
   single group, checked against its root. Outboards are about 0.4 % of the
   blobs (64 bytes per 16 KiB): 4 MB for 1 GB, none for a stick of small
   files. The catalogue's record wins over the outboard: an outboard is
   used only if its length is the one its blob's size gives (parents × 64);
   one that is short, or fails its root, is thrown away and made again
   (rule 4), without forgetting the file's record. The pass removes the
   outboards of blobs no longer in the complete version it publishes, once
   no transfer of them is in flight (on Windows an open file cannot be
   removed).
2. **Bytes are made when asked.** Our store's `ExportBao` gives bao-tree's
   `traverse_ranges_validated` three things, **all from the catalogue's
   record, never from the file**: the blob's size (`blob_len` of the
   recorded plaintext size), its root (the manifest's hash) and its
   outboard, read from its file (bao-tree's file-backed outboard, not a
   copy in memory: a 100 GB file has a 400 MB outboard). The data is a
   reader over the blob that reads the file's plaintext at the needed
   offset and encrypts those 64 KiB chunks as `encrypt_blob` would (header,
   nonce prefix from the recorded plaintext hash, ADR 0003; chunk `i` at
   `header + i × (64 KiB + tag)`). A file shorter than its record is a read
   error; bytes past its recorded size are never read. The reader keeps the
   last chunk it made, so the four 16 KiB groups of one chunk do not encrypt
   it four times.
3. **No byte that differs from the hashed blob ever leaves.** bao-tree
   0.16.1 hashes every 16 KiB group (whole or in part of a range) and every
   parent against the tree **before** sending it, and sends the very buffer
   it checked (`io/mixed.rs`, leaf and parent checks before `send`). So:
   groups are sent while they match; the first group that does not is
   never sent, and the transfer stops there. A file changed since the
   catalogue gives other bytes under the same nonce prefix only in the
   groups it changed, and those never leave: no nonce is used for two
   plaintexts outside the owner (ADR 0003's guarantee, which suitcases
   kept by deleting what they wrote). The size sent first comes from the
   record (rule 2), so a file that grew or shrank does not tell its new
   size either.
4. **What a mismatch does.** The store tells `Serving` (that blob is "not
   ready" from then on) and the owner's pass (a `Demand` variant), which
   forgets the file's record and reads it again: a reader is told "not
   ready" until the next version lists the file anew, instead of failing
   on it again and again. Making a missing outboard or plaintext hash
   (rules 1 and 5) reads the file as it is now, and keeps the result only if
   the blob hash it gives is the manifest's; otherwise the record is
   forgotten as for a mismatch. So a changed file never gets an outboard of
   its own under the old nonce prefix.
5. **A file needs its plaintext hash and its outboard to be served.** The
   catalogue records the plaintext hash for every file it reads. A record
   without one, or a blob of more than 16 KiB without its outboard (a cache
   from before this ADR, a cleared cache), is "not ready" and is made by the
   owner's background work (rule 7), first for blobs readers wait on.
6. **Errors while serving are "not ready", not failures.** A read error (a
   disk that sleeps or is unplugged, a linked folder that went: ADR 0008)
   ends the transfer, and the next ask for that blob is "not ready" while
   its root is gone, as today; the reader waits and asks again. The other
   store commands are answered with an "unsupported" **error**, never an
   empty success (an empty answer must mean refusal).
7. **What remains of the keeper.** A small queue in the owner's sharing
   task: blobs readers asked for that are "not ready" (missing outboard or
   plaintext hash, rule 5) are made first, then the rest of the stick in
   manifest order, through the disks' turns (ADR 0013, its rule 5 for a
   reader waiting). After an upgrade from ADR 0007's caches every blob is in
   that state, so a reader's first download of an old stick waits for this
   once (the owner shows "getting files ready", as it does for a suitcase
   today); later restarts find the outboards on disk.
8. **Reading for transfers is bounded and in order.** File reads and
   encryption for transfers run on a pool of a fixed size for the whole
   program (4 threads), not tokio's default blocking pool (up to 512). A
   transfer takes a disk turn (ADR 0013) per window, not per chunk, so one
   turn serves a run of a file; with ADR 0016's 32 asks at once, asks are
   served in manifest order so one hard disk reads mostly forward.
9. **What goes away:** suitcases and their preparation, eviction and
   budget (`share/keeper/` but the queue of rule 7,
   `transport/iroh/suitcases.rs`); the `Publisher` trait's `open_suitcase`,
   `add_to_suitcase` and `evict`; the owner's encrypted cache and its
   space claim (`space_claims` for owners) and the cache estimate before
   `share`; Amendment 2. At the first start after this, old suitcase
   folders and the owner's cache are removed and its space claim released.
   Not `threads.rs`: the reader's batch `FsStore`s still need it.

## What it costs

- **CPU and disk reads per transfer.** Each blob sent is read and encrypted
  again, for every reader (XChaCha20-Poly1305 runs at about 1–2 GB/s per
  core; the network is slower). A suitcase did it once per preparation, but
  was prepared again after it left; with several readers at once, the
  owner's disk reads the same files several times (bounded by rule 8).
- **Code against a semi-internal API.** Our actor must answer every
  iroh-blobs store command (the `Command` enum is generated from
  `proto::Request` and is not `#[non_exhaustive]`). A new iroh-blobs
  version may add commands and break our build; the version stays pinned
  and the upgrade checklist names this file.
- **A direct dependency on `irpc`**, the version iroh-blobs uses, to name
  its client and channel types (needs Giorgos's approval; it is already in
  the tree through iroh-blobs).
- **New code:** the outboard writer in the catalogue, the encrypting reader
  (random access into the blob format), the store actor, outboard cleanup,
  the queue of rule 7. Removed: suitcases, most of the keeper, the owner's
  budget.

## Not in this ADR

- **The reader.** It still downloads into a batch `FsStore` per 512 files
  (ADR 0007 rule 3) and reached 200–240 MB for 100 000 files on all three
  systems. The same idea (a store of ours that writes straight to the
  file) may fit it, in an ADR of its own.
- **glibc's arenas.** With one store and a bounded pool, the owner should
  make few threads. If Linux still keeps freed memory after this, the
  `mallopt` change (set aside, `docs/todo.md`) is measured again then.

## The prototype (2026-10-05)

A test build (switched on by `OURSTICK_PROTO_MADE`, outboards made when
first asked and kept in memory) against today's, same PC and data, one
local reader (two for that row):

| | today | prototype |
|---|---|---|
| Linux, owner, 100 000 files | 243 → 377 MB at 25 000 files, rising | 196 → 279 MB at 20 000, then 236–241 MB to the end |
| Linux, owner threads | 15 → 64 | 9–10 |
| macOS, owner, 100 000 files | 124 → 206 MB, 8 → 208 threads | 128 → 141 MB, 11 threads |
| macOS, whole stick | 526 s | 526 s |
| Linux, 10 000 small files | 136–207 s | 123–142 s |
| Linux, one 2 GB file | 101–134 s | 89–97 s (outboard made at the request) |
| Linux, two readers of 10 000 | 291 s | 209 s |

Three things were learned on the way: a blob of one 16 KiB group needs no
outboard and must not have one made (it read the file twice); the keeper's
per-file state and the store's table were the same list twice; the store's
table should keep each file's manifest path and the index of its root, not
its whole path on disk (about 90 MB for 100 000 files with whole paths).
The catalogue's cost of making outboards is measured in step 1.

## Order of work

0. **Prototype, measured first** (done 2026-10-05, above; the store actor form
   passed, so the fallback below was not built).
   Against today's build, on the same PC and data, it passes if:
   - **memory:** the owner of the 100 000-file stick does not grow with the
     files it serves: once warmed up (after its first 20 000 files), its
     RSS stays within 10 %, over the whole stick, on Linux without an arena
     limit and on macOS. (First written "within 10 % of its RSS at the start
     of serving"; the prototype showed a one-time rise as its few threads
     start working, which does not grow with the stick.)
   - **threads:** at most 32 at any time on the owner;
   - **speed:** 10 000 small files and one 2 GB file download no slower
     than 1.2 × today, cold and warm, on Linux and macOS;
   - **two readers at once** on the same stick finish no slower than 1.2 ×
     today;
   - **the catalogue**, building outboards, is no slower than 1.1 × today.
   Otherwise this ADR is withdrawn.
1. (done) Outboards in the catalogue, written atomically and cleaned up; tests.
2. (done) The encrypting reader; tests.
3. (done) The store actor (or the fallback) and `ExportBao`; two peers get a
   folder through it.
4. (done) `Serving` and the handler use it; "not ready" for missing outboards,
   plaintext hashes, mismatches and read errors; the queue of rule 7.
5. (done; measured, below)
   Suitcases, most of the keeper and the owner's budget removed; the
   in-memory `Publisher` of the tests loses its suitcase calls (it keeps its
   own notion of "ready", with no encryption path; the real store is tested
   against a real iroh-blobs fetch); ADR 0007, 0016 rule 6 and
   `docs/protocol.md` updated; the measurements again on all three systems.

## Notes from the implementation (2026-10-05)

- The queue of rule 7 is `share/files_keeper` (`keep_files`); `Publisher`
  gained `serve_files` and `serve_more`, and `Demand` gained `NotServed`.
- A read error or mismatch while a blob is made ends the transfer with
  `ERR_INTERNAL` on the wire (`TransportError::OwnerFailed` in the reader).
  The reader treats it as "keeps changing": a short wait, then asks again,
  not "damaged".
- The owner keeps no space claim: migration 0021 deletes old `cache` rows.
  Old `cache/<stick>/suitcases` folders are removed when sharing starts.
- **Not as rule 8 says, at first:** transfers took no disk turn and asks
  are not put in manifest order. Amendment 1 (below) built the turns.
- **Not as rule 1 says:** outboards of blobs no longer in the version are
  removed right after it is published, not once no transfer of them is in
  flight. On Unix a transfer keeps reading the removed file; on Windows the
  removal fails, is logged, and is tried again after the next version.
- Before a transfer begins, the owner looks at the file and its outboard
  (`Probe`): gone, or not of its recorded size, it is "not ready" at once
  instead of a transfer that fails.
- An outboard on disk that differs from the one a new read of its file
  gives (cut short by a crash, damaged) is replaced.

## Measured after step 5 (2026-10-05/06)

The 100 000-file stick, owner and reader on one machine, the reader from
scratch to the end (Windows stopped at 70 000 files: the host ran out of
memory and Defender scanned every file written).

| | Owner memory | Owner threads | Reader peak | Download |
|---|---|---|---|---|
| Linux, before (suitcases) | 296 → 377 MB at 25 %, growing | 5 → 64 | 490 MB | |
| Linux, glibc default | 282 → 311 MB, growing slowly | 11 | 490 MB | 38:19 |
| Linux, two arenas (`mallopt`) | 250–258 MB, flat | 15 | 281 MB | 38:50 |
| macOS | 134 MB, flat | 16 | 206 MB | 8:37 |
| Windows (VM), before | 106 MB at 30 % | 74 → 82 | 89 MB at 30 % | |
| Windows (VM) | 106–109 MB, flat | 20–22 | 111 MB at 70 % | |

On Linux the heap peaked at 176 MB (170 MB in the prototype): the rest was
glibc's arenas, which `threads.rs` now limits to two. The criteria of
step 0 hold on all three systems.

## Tests to write

- Outboard: for sizes 0, 1, 16 KiB ± 1, 64 KiB ± 1 and several MB, the
  outboard and root match those iroh-blobs computes for the same blob; a
  short or wrong outboard is thrown away and made again, the record kept.
- Encrypting reader: every range read, including ranges starting inside a
  chunk and a group, equals the same range of `encrypt_blob`'s output.
- A file changed after its catalogue (one byte in the middle, a
  truncation, a longer file), for a blob of at most 16 KiB and a big one:
  the reader never receives a byte that differs from the original blob, nor
  the new size; the blob is "not ready" afterwards and the record
  forgotten; the next version serves the new file.
- Making a missing outboard for a file that changed: no outboard kept, the
  record forgotten.
- A file that vanishes or fails to read mid-transfer (a linked folder
  removed): the transfer ends, the next ask is "not ready".
- Two peers, many small files and one big file, ranges and resume (a
  reader that stops halfway gets the rest by range); two readers at once.
- An old cache (suitcases, no outboards): removed at start, its claim
  released, every blob made once and served.

## Alternatives considered

- **Keep suitcases, evict idle ones (Amendment 2).** Measured: fixes
  macOS, not Linux without an arena limit; keeps the disk budget, the first
  wait and the re-reads.
- **Ask iroh-blobs for a shared runtime** (`docs/upstream/`). Would cut the
  threads, but keeps suitcases, the budget and the copies; and it is not in
  our hands.
- **Our own transfer protocol.** Everything above plus verified streaming,
  ranges and resume of our own, and every link and reader made so far stops
  working. Not worth it while iroh-blobs serves from a store of ours.
- **Write the bao response ourselves instead of a store actor** (our
  handler already owns the stream). No semi-internal API, but our code then
  has to match iroh-blobs' wire format. Not built: the store actor passed;
  it stays the way out if a new iroh-blobs breaks the actor.
- **mimalloc or another allocator.** Measured worse on macOS (3–4×) and
  Windows (1.5–2.5×).

## Amendment 1: transfers read through the disk's turns (accepted 2026-10-06; built, to be measured)

Rule 8 is not built: a transfer reads its file on the 4 threads outside
ADR 0013's turns. While it sends, the owner does not give way to another
program on that disk, and its reads are not judged, so a reader that keeps
the owner busy can slow the user's own work on a hard disk without
OurStick ever noticing. This amendment says how rule 8 is built. Reviewed
by Fable and Sonnet (2026-10-06); their notes are in the text.

1. **One transfers' reader per stick and disk.** All transfers of a stick
   from one disk are one reader of that disk, beside its catalogue and its
   files keeper. It is a new kind of reader, shared by several threads at
   once (`SharedReader`: a turn, a window and a count of reads in flight
   under one lock). ADR 0013 rule 5 has one `DiskReader` per thread, since
   two readers of one task would wait for each other; transfers are not
   one task, and they share one turn instead of each taking one. A stick
   whose linked folders are on other disks has one per disk
   (`DiskId::of_file` of each blob's file). A read takes one disk's turn
   only, so two disks' turns never wait for each other.
2. **The turn is taken by the first read and kept while it is useful.** A
   chunk read (64 KiB, in `MadeBlob::chunk`, after the last-chunk cache:
   a cached chunk reads nothing and takes no turn) joins the transfers'
   turn if it is held, or takes one, **urgent** (readers wait on it).
   When the last read in flight ends, the turn is kept, idle; the next
   reader in the disk's queue takes it after 50 ms idle (`IDLE_GRACE`,
   unjudged). So a transfer waiting on the network keeps the catalogue,
   the keeper or another stick from the disk for at most that long, and
   transfers reading steadily keep their turn for a whole window, not one
   chunk (one chunk then a 64 MB catalogue window would leave a reader
   ~400 KB/s on a hard disk). *As built (after review):* a quantum of
   8 MB or 250 ms, first proposed, was dropped: it ended the transfers'
   turns before their window, unjudged, so the catalogue got the larger
   share, and the idle grace already bounds a network wait.
3. **Its window.** The window is kept across turns and judged once, when
   it is full: 64 files begun (a file begins at a transfer's first chunk
   read of its blob) or 64 MB read. A full window takes no new read: the
   reads in flight end it, so reads that always overlap still give the
   turn back. A turn given back before then is not
   judged and sets no pause (`give_back` gains that path). Its time is the
   wall time some read was in flight while the turn was held (a new
   `Window` field), not the sum of the reads, which overlap; the pause is
   taken from it too. Every transfer read counts as a first read: a
   reader asks each part once.
4. **Judged as its own kind.** Four reads at once at scattered offsets of
   32 files cost a hard disk differently from the catalogue's forward
   reads. `DiskShare` keeps its learned speeds per kind of reader
   (catalogue and keeper; transfers), and one judgement per disk (giving
   way, slow windows in a row), so a slow transfers' window makes all of
   OurStick give way, and neither kind is judged by the other's speed.
5. **Urgent, three windows to one.** `URGENT_IN_A_ROW` counts judged
   windows of urgent readers (in `give_back`), not turns granted: the
   transfers' reader takes many short turns. The keeper making a file a
   reader waits on is urgent too; it and the transfers take urgent turns
   in the order they asked. Another stick's catalogue on that disk gets
   one window in four while readers wait; accepted.
6. **Giving way.** A turn starts only after the disk's pause, as for any
   reader: while OurStick gives way, transfers read a tenth of the time,
   and readers see a slower owner (ADR 0013's intent). Nothing on the
   wire changes.
7. **Waiting holds a made-blobs thread, nothing else.** A read waiting for
   a turn blocks one of the 4 threads; the turn is never held across an
   await, so giving it back never needs a runtime. The store's actor
   (`made_store::commands::run`) moves to the program's runtime (it does
   nothing blocking), so new asks, "not ready" and `shutdown` are
   answered while exports wait. Four transfers waiting on one disk stall
   transfers from other disks for at most `LONGEST_PAUSE` (30 s) at a
   time; accepted (one owner shares from one disk in nearly every case).
8. **Stopping.** Each read asks `go_on` first and while it waits: the
   store's shutdown. *As built:* not its transfer's end; a read waiting
   for a turn after its reader went away reads its chunk when the turn
   comes, and the transfer ends at its next send. A read that leaves the
   queue, or fails, is counted out by a guard in the read itself, so the
   turn is never left held.
9. **Not covered.** Outboards are read from the cache folder (often
   another disk) outside the turns: 0.4 % of a blob. The order of the 32
   asks (rule 8's second half) is left out: the reader asks in manifest
   order, and windows keep reads near each other; built only if measured.
10. **Where it lives.** `disk_turns` and `disk_share` move from `share/`
   to `crate::disk` (`manifest/hashing.rs` already uses them), and
   `disk_turns::Disks` is renamed `DiskTurns` (`space::Disks` is another
   type). `IrohPublisher::start` takes the device's `DiskTurns` once.
   `MadeBlob` reads through a small `Turns` handle, a no-op in its tests.

**Tests.** On a fake disk: a transfer and a catalogue take turns (the
catalogue reads between transfer windows, at most three to one); a
transfer waiting on the network gives the turn back to a waiting
catalogue within the quantum, and keeps it when nobody waits; a window
kept across turns is judged once, with wall time of overlapping reads;
a cached chunk takes no turn; while giving way, transfers wait for the
pause; a read cancelled in the queue or in the pause leaves no turn held;
reads on two disks at once; a slow transfers' window makes the catalogue
pause too; `shutdown` answers while exports wait. Two peers still get a
folder whole.

**Measured** on this PC's hard disk (G), page cache dropped, median of 3:
the 100 000-file stick and one 4 GB file, owner and reader on this PC.
- Alone: transfer time within 10 % of today's (`566d7bf` build).
- With a reader on a slow link (relay or throttled), a catalogue of
  another stick on G finishes within 10 % of its time alone.
- With a copy of a big file on G beside the transfer: the copy takes at
  most 1.3× its time alone (ADR 0013's criterion), and OurStick says it
  is giving way.
