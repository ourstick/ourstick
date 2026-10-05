# ADR 0017: Blobs made when asked, from the files themselves

**Status:** proposed — 2026-10-05. Would supersede ADR 0007's suitcases for
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

## Order of work

0. **Prototype, measured first**, in two forms side by side: the store
   actor of rule 2, and the fallback below (our handler writes bao-tree's
   encoding to the stream itself). The smaller one that passes goes on.
   Against today's build, on the same PC and data, it passes if:
   - **memory:** the owner of the 100 000-file stick stays within 10 % of
     its RSS at the start of serving, over the whole stick, on Linux
     without an arena limit and on macOS;
   - **threads:** at most 32 at any time on the owner;
   - **speed:** 10 000 small files and one 2 GB file download no slower
     than 1.2 × today, cold and warm, on Linux and macOS;
   - **two readers at once** on the same stick finish no slower than 1.2 ×
     today;
   - **the catalogue**, building outboards, is no slower than 1.1 × today.
   Otherwise this ADR is withdrawn.
1. Outboards in the catalogue, written atomically and cleaned up; tests.
2. The encrypting reader; tests.
3. The store actor (or the fallback) and `ExportBao`; two peers get a
   folder through it.
4. `Serving` and the handler use it; "not ready" for missing outboards,
   plaintext hashes, mismatches and read errors; the queue of rule 7.
5. Suitcases, most of the keeper and the owner's budget removed; the
   in-memory `Publisher` of the tests loses its suitcase calls (it keeps its
   own notion of "ready", with no encryption path; the real store is tested
   against a real iroh-blobs fetch); ADR 0007, 0016 rule 6 and
   `docs/protocol.md` updated; the measurements again on all three systems.

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
  has to match iroh-blobs' wire format. Built side by side in the
  prototype (step 0).
- **mimalloc or another allocator.** Measured worse on macOS (3–4×) and
  Windows (1.5–2.5×).
