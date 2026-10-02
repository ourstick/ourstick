# 0014: Receiving many small files

Status: Proposed (2026-10-03)

## Context

The first scale test (`docs/todo.md`, 2026-10-03): 100 000 files of 1–8 KB,
CLI, Linux to the same Linux, on an NVMe disk. Every file came, identical,
but the first `get` took 17:51, about 93 files a second, with both sides
idle three quarters of the time.

Timed on 10 000 such files (112 s in all; timers added for the measurement,
not kept):

| Step of each file | Time | A file |
|---|---|---|
| Recording it (`record_downloaded_file`, one transaction each) | 53 s | 5.3 ms |
| Waiting for it to be placed (decrypt, `sync_all`, rename), one at a time | 35 s | 3.5 ms |
| Fetching its blob | 15 s | 1.5 ms |
| Checking its path, asking the blob store | 3 s | 0.3 ms |

The same files with the database's commits not synced: 63 s. With no
syncs at all: 15.5 s. The network is not what is slow: the syncs are, one
after another. On this machine one `fsync` of a new 4 KB file costs
4.6 ms on the NVMe disk and 36 ms on the spinning one (`sda`); from 16
threads at once they cost 0.56 ms and 10 ms a file, because the
filesystem joins syncs that wait together into one journal commit.
`Rescued_Disk` has 146 297 files: on a spinning disk, one sync
at a time is about 85 minutes of syncing alone.

The syncs are there for a reason, and stay. A file is synced before it is
moved into place, so a power cut never leaves a short or empty file at a
place OurStick would then take for one the user changed (and leave alone,
reported, forever). A record says "this file is here as written", so it
is written only after the file is.

## Decision

1. **Records are written in groups.** The files placed (and the files
   found already here, `FolderPlan::found`) are recorded in one transaction
   for up to 256 files, and at the end of each batch (ADR 0007), before the
   batch's blobs are dropped, on an error, and when the user stops. A
   record is still written only after its file is synced and in place.
   A power cut loses at most the records of one group: those files are in
   place, synced and unrecorded, so the next round compares them by
   content (`docs/protocol.md`, reader-side rules), finds them equal, and
   records them. Nothing is fetched again, and nothing is taken for a file
   the user changed.
2. **Up to 16 files are placed at once.** Each is still decrypted, synced
   and moved into place on its own, in the order it came; up to 16 of them
   are in flight together, each on a blocking thread, while the next blobs
   are fetched. Their syncs then wait together and the filesystem joins
   them. A file is told done (`FileDone`) and recorded in the order it
   came, so events and records stay as today, only later. A file whose
   blob is the same as one in flight (identical files, ADR 0003) waits for
   that one, as today, then is copied from it.
3. **Nothing on the wire changes**, nor what is kept, nor where: only when
   syncs and records happen. The owner's side is not changed: its
   catalogue already records 256 files a transaction, and serving does not
   sync.
4. **Fetching stays one blob at a time** for now. It is 1.5 ms a file
   here (more across a real network: a round trip each), and fetching
   several at once is a change of the transport (iroh-blobs requests in
   parallel, or one request for a batch's blobs). It gets its own ADR if,
   after this, it is what is slow.

## In the code

- `Store::record_downloaded_files(&StickId, &[DownloadedFile])`: one
  transaction, the same checks as `record_downloaded_file` (a stick being
  forgotten, two files at one place) for every row, all or nothing. Both
  stores; `record_downloaded_file` stays for single files.
- `download/fetch_files.rs`: `placing: Option<Placing>` becomes a queue of
  at most `PLACING_AT_ONCE = 16`, finished from the front; `Done` keeps the
  rows to record and writes them at 256 (`RECORD_GROUP`), at the end of a
  batch, and on every way out (the existing "on an error too" finish).
- `download/joined.rs`: the found rows in groups too.
- Tests: the queue keeps order and caps at 16; a twin waits for its
  source in the queue; records are written in groups and all are written
  on an error and a stop; a store refusing a group (stick being forgotten)
  fails the round as one refused record does today; a "power cut" test
  (records of the last group lost, files in place) finds them equal by
  content and records them without fetching.
- Measured again with the timers, 10 000 and 100 000 files, NVMe and
  `sda`.

## Consequences

- Expected, from the measurements: about 15–20 s for the 10 000 files
  here instead of 112, so roughly six times faster on many small files; on
  a spinning disk, the syncs about a quarter of today's time. Big files do
  not change: one sync among gigabytes is nothing.
- Up to 16 files are being written at once: 16 partial files in the
  receiving folder's hidden partial folder instead of one, and 16 blocking
  threads. The blobs they are decrypted from are in the batch's store
  already; no more room is used than a batch's.
- After a crash, up to 256 files are compared by content once more. On a
  folder of photos that is a few seconds.
- The window's progress moves in small steps, as files finish, as today.

## Order of work

1. `record_downloaded_files` on both stores, with tests.
2. Records in groups in `fetch_files` and `joined` (placing still one at a
   time), with tests; measure.
3. The queue of 16, with tests; measure.
4. Docs: protocol.md (when records and syncs happen; nothing on the wire),
   todo.md; status Accepted. Then the scale test again: 100 000 files to
   Windows and the Mac with the app.
