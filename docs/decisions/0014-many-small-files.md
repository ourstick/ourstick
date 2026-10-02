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
   records them; a file that replaced an older recorded version is found
   equal too (its content is the manifest's, never "changed here"). Nothing
   is fetched again. One rare case loses a little: a file saved under
   another name (ADR 0010) whose record was lost has no claim on its place
   until it is recorded again; if a new version brings, before that, a
   file whose own name is that place, the new one takes it, the old one is
   reported changed there and fetched again under a numbered name. Today's
   single records make it a window of one file; groups, of 256.
   One row that cannot be recorded (another file's place, a stick being
   forgotten) fails its whole group and the round, as one refused record
   fails the round today; the group's other files are not recorded, and
   are found equal by content at the next round. Both stores check every
   row before writing any.
   Grouping matters most for the files found already here: a stick
   received again over 100 000 files in place records 100 000 rows before
   it fetches a byte, about 9 minutes of synced commits today, a second
   in groups.
2. **Up to 16 small files are placed at once.** Each is still decrypted,
   synced and moved into place on its own, in the order it came; up to 16
   of them are in flight together, each on a blocking thread, while the
   next blobs are fetched. Their syncs then wait together and the
   filesystem joins them. Only files of at most 4 MB join the queue, and at
   most 64 MB are in flight in all: a bigger file waits for the queue to be
   empty and is placed alone, as today (its one sync is nothing beside its
   bytes). So a stop (Pause, Quit, Forget) waits at most for 64 MB of small
   files to be placed, a second or two, plus the one big file it waits for
   today.
   A file is told done (`FileDone`) and recorded in the order it came, so
   events and records stay as today, up to 16 files later; the window needs
   no change. A file whose blob is the same as one in flight (identical
   files, ADR 0003) waits until the last in flight with that blob is
   placed, then is copied from it, as today; before it is fetched as well
   (a twin whose copy fails is fetched, and fetching clears that blob's
   leftovers in the partial folder).
   On every way out (an error, a stop), every file in flight is waited for,
   told done, and its row written in the last group; then the first error
   is returned. No placing is left running after the round returns.
3. **Nothing on the wire changes**, nor what is kept, nor where: only when
   syncs and records happen. The owner's side is not changed: its
   catalogue already records 256 files a transaction, and serving does not
   sync.
4. **Rejected: the database's commits not synced** (`PRAGMA synchronous
   = NORMAL` in WAL), one line for the same 53 s. It is per connection, and
   the one connection holds the device's and the sticks' rows too, whose
   loss after a power cut can end shared links (CLAUDE.md rule 9). A second
   connection only for records is more code than groups. Rejected too:
   placing without syncs and syncing a whole directory or filesystem
   (`syncfs`) at a batch's end: an unsynced file could be in place, short,
   after a power cut, and `syncfs` is Linux only.
5. **Fetching stays one blob at a time** for now. It is 1.5 ms a file
   here (more across a real network: a round trip each), and fetching
   several at once is a change of the transport (iroh-blobs requests in
   parallel, or one request for a batch's blobs). It gets its own ADR if,
   after this, it is what is slow.

## In the code

- `Store::record_downloaded_files(&StickId, &[DownloadedFile])`: one
  transaction, the same checks as `record_downloaded_file` (a stick being
  forgotten, two files at one place) for every row, all or nothing. Both
  stores; `record_downloaded_file` stays for single files.
- `download/placing_queue.rs` (new; `fetch_files.rs` is near its size
  already): the queue, finished from the front, with `PLACING_AT_ONCE =
  16`, `QUEUED_FILE_BYTES = 4 MB` and `QUEUED_BYTES = 64 MB`, and a count of
  blobs in flight by hash for twins. `Done` keeps the rows to record and
  writes them at `RECORD_GROUP = 256`, at the end of a batch, and on every
  way out. 16 is where the measured syncs stopped getting much cheaper on
  this machine's disks (4.6 → 0.56 ms at 16, 0.23 at 64); 256 is the
  owner's catalogue group; both are measured again in steps 3 (Linux) and 4
  (the Mac, Windows).
- `download/joined.rs`: the found rows in groups too.
- Tests: the queue keeps order and caps at 16; a twin waits for its
  source in the queue; records are written in groups and all are written
  on an error and a stop; a store refusing a group (stick being forgotten)
  fails the round as one refused record does today; every placing is
  awaited on a stop; big files are placed alone. A "power cut" test: a
  whole download, then the last group's records removed
  (`forget_downloaded_file`), then a second round, which must fetch
  nothing and record them all (it tests the recovery, which is what a lost
  group needs).
- Measured again with the timers, 10 000 and 100 000 files, NVMe and
  `sda`.

## Consequences

- Expected, from the measurements: about 18–25 s for the 10 000 files
  here instead of 112 (fetching and checks, 18 s, stay one at a time), so
  four to six times faster on many small files; on a spinning disk, the
  syncs about a quarter of today's time. On a Mac, `sync_all` is a full
  flush of the disk's cache (`F_FULLFSYNC`), and on Windows
  `FlushFileBuffers` flushes the volume's: they may join less than on
  Linux, so the gain there is measured before this is accepted; records in
  groups help there in any case. Big files do
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
3. The queue, with tests; measure (10 000 and 100 000 files, NVMe and
   `sda`, the timers of the Context).
4. Measure on the Mac and on Windows (the VM): 10 000 small files from
   Linux, today's build and this one.
5. Docs: protocol.md (when records and syncs happen; nothing on the wire),
   schema.md ("a row is written after each file is in place": now in
   groups, still after), todo.md (the numbers); status Accepted. Then the
   scale test again: 100 000 files to Windows and the Mac with the app.
