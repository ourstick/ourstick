# 0013: Giving way to the user's disk

Status: Proposed (2026-10-03)

## Context

A big stick is read for hours. The first test of scale is
`Rescued_Disk`: 980 GB in 146 297 files on a spinning disk
(this Linux machine's `/media/<user>/G`, `sda`). The owner reads each file
three times before a reader has it:

- the catalogue (ADR 0007) encrypts every new file in full only to hash the
  blob, and `encrypt_blob` reads it twice (once to derive the nonce from the
  plaintext's hash, ADR 0003, once to encrypt);
- a suitcase reads it once more (`encrypt_blob_hashed_before`), writing the
  blob into the data folder.

At about 120 MB/s that is roughly 4–5 hours of catalogue on that disk, and
the suitcases come on top, one gigabyte at a time while readers fetch.
While OurStick reads, the disk is not the user's: a spinning disk serving
two sequential readers seeks between them, and each gets a small part of
the speed it had alone. Copying photos off a card, opening a video, or a
backup crawls, and the user blames the computer, then OurStick.

The obvious answer, OS I/O priority, is not available:

- on this machine the HDD's scheduler is `mq-deadline`, which ignores
  `ionice` classes (only BFQ honours them, and the user chose neither);
- Windows' background I/O mode (`SetThreadPriority` with
  `THREAD_MODE_BACKGROUND_BEGIN`) and macOS's (`setiopolicy_np` with
  `IOPOL_THROTTLE`) are system calls without a safe wrapper in our
  dependencies: `unsafe` code, which CLAUDE.md allows only with Giorgos'
  approval, for one platform each;
- none of them knows that a reader is waiting for exactly this suitcase.

What exists today: while readers were told "not ready" in the last minute,
the catalogue pauses after each part as long as the part took, at most
10 s (`share/pass.rs`, `LONGEST_YIELD`), so the suitcases for them go first.
Nothing gives way to anyone outside OurStick.

## Decision

1. **OurStick times its own reads from the disk, and slows down when they
   get slower.** Only the time spent inside the file's `read` calls counts
   (a timing adapter around the file), not the encryption or the hashing
   in between (about 195 MB/s on this machine, ADR 0007, so a window's whole
   time would hide half of a slowdown), nor recording or publishing. Only
   the first read of a file counts: the second read of `encrypt_blob` comes
   from the page cache for a file that fits in memory and from the disk for
   one that does not, so it says nothing steady about the disk. Files taken
   from their record (not read) do not count.
2. **Speed is judged against similar work, in windows.** A window is
   64 MB read, or 64 files, whichever comes first, also inside one big file.
   Small files cost a seek each, whatever their size; big ones cost their
   bytes. A window is judged by what it should have taken at this disk's
   recent best: a cost per file and a cost per byte, each the best of the
   recent windows of that kind (windows of mostly big files teach the cost
   per byte, windows of many small ones the cost per file). The recent best
   fades, by half over an hour of reading time (the sum of timed reads: no
   wall clock, so tests are exact), so a disk whose inner tracks are
   slower does not count as "someone else" for long. **A window faster
   than any disk teaches nothing:** files just copied into the folder are
   still in memory, and a window read from there (over 1 GB/s, or under
   0.2 ms a file) would set a "best" no disk can keep, after which every
   window from the disk looked slow. Such a window counts as fast (rule 3),
   but is not a best. And what a window should take is the **second** best
   of its kind, not the best: one window read partly from memory (a few
   files just copied in, under the floor) is not a speed the disk keeps.
3. **Someone else is there** when two windows in a row took more than
   three times what they should have. Three, not two: the inner tracks of a
   disk are up to half as fast as its outer ones, and a second sequential
   reader on a spinning disk costs far more than that. **They are gone**
   when one window takes less than one and a half times what it should
   have. One slow window alone (a drive waking up, a sector read again, a
   flash drive pausing to write) is never enough.
4. **Giving way:** while someone else is there, after each window OurStick
   pauses nine times as long as the window took (it reads a tenth of the
   time), at least 1 s, at most 30 s. The pause is in the window, so a 30 GB
   file gives way as often as a folder of photos, and the user's program
   waits at most one window (64 MB, under a second on a spinning disk) for
   the disk.
5. **One place decides per disk.** All of OurStick's reading from one disk
   goes through that disk's `DiskShare`, whichever stick and whichever
   reader (a catalogue, a suitcase) it is for. A "disk" is a filesystem:
   its device id on Linux and macOS, its volume on Windows (the path's
   drive letter, `C:` for `C:\` and `\\?\C:\` alike); one share for the
   whole process where neither is known. OurStick reads a stick by its
   folders' own paths, never through a `subst` letter (the app's drive is
   one, for the user), so a letter is not resolved. Two partitions of one
   physical disk are two filesystems, as are APFS volumes of one container
   and btrfs subvolumes: OurStick on one may take OurStick on the other for
   someone else; rare, and it only slows both. Only one reader reads a window at a time: they take turns in
   the order they asked (a queue, not a lock, so a catalogue taking turn
   after turn cannot shut out a suitcase or another stick), so OurStick
   never mistakes itself for someone else. When readers wait for a suitcase
   (today's "not ready"), the suitcase goes first, but for at most three
   windows for each window of the catalogue, so new files still reach
   someone (what `LONGEST_YIELD` bounds today); this replaces it. The pause
   is the disk's, not the reader's: it is a time before which the next turn
   does not start, whoever takes it. The reader whose turn it is sleeps it
   holding the turn (no one would read meanwhile anyway, and the order is
   kept), asking every 5 s whether to go on; one waiting in the queue asks
   every half second, so a stick that stops leaves the queue at once. A
   reader reads one disk through one `DiskReader` per thread (two of one
   disk in one thread would wait for each other), and gives its turn back
   at the end of each part, before recording or publishing. A suitcase
   gives way to the user as well: a reader who waits longer because the
   sender's disk is busy is told nothing new ("the sender is getting it
   ready" is what they already see). Two programs of
   OurStick on one disk (the CLI and the app) do not know of each other;
   they are rare, and each then gives way to the other as to anyone.
6. **The owner sees it,** once when it starts and once when it stops, not
   at every window: the CLI prints "Reading your folder slowly, so another
   program can use the disk." and "Reading at full speed again."; the app
   shows the first under a sharing stick's state while it lasts. No
   setting: a stick that never gives way is not something a user asks for,
   and one that always crawls is what this avoids. Each change is logged
   (`debug`) with its numbers (the window's time, the expected time), never
   a path.
7. **Not the reader's side, not now.** A receiver writes at the network's
   speed, which is far below a disk's except on a fast home network; files
   already in its folder are compared by content only when they are not as
   it recorded them (a folder received before without records, or files
   changed there). That can be long (a 500 GB folder received again in a
   new place), and the todo says so; it can use the same `DiskShare` later.

## In the code

- `share/disk_share.rs` (new, pure, no I/O): `DiskShare` with
  `after_window(files, bytes, took: Duration) -> Duration` (the pause),
  `giving_way() -> bool`, and the named constants of rules 2–4 in one place.
  Its time is what it is given (`took`), so tests drive it with numbers
  only, no `Clock`.
- `share/disk_turns.rs` (new): one `DiskShare` per disk per process, behind
  the queue of turns of rule 5, and `Timed<R: Read + Seek>`, which times the
  first read of a file (the seek back to its start, as `encrypt_blob` does,
  ends it), closes windows, and waits for the disk's pause in the blocking
  thread it reads in. The catalogue opens files itself
  (`hash_file_telling_plaintext`), so it is given a way to open a file as a
  `Timed` reader, not only the `DiskShare`. The second read (after the
  seek back) still takes turns and pauses at each window like the first,
  so a file bigger than memory, read from the disk twice, still leaves the
  user nine tenths of the time and never reads out of turn; it only does
  not feed the judgement (no best, no slow or fast window). A drive found
  missing between slices of a pause comes out of `read` as an error of its
  own kind, which the catalogue passes on as `DriveMissing`, not as a file
  gone. The `DiskShare`s and the way to open a `Timed` reader are given to
  the catalogue (`manifest/catalogue.rs`, through `next_part`) and to the
  keeper (`share/keeper/prepare.rs`) by the caller, not made inside.
  `Timed` measures and sleeps through a small trait of its own (`Timer`:
  monotonic elapsed time, sleep), real in the app and the CLI, a fake in
  tests, so the bounds of the pause and the ejected-drive slices are tested
  without waiting. It is not the core's `Clock`, which tells wall time in
  milliseconds for records (and can jump); a pause needs a clock that never
  goes back.
- A trait for the judgement (`DiskLoad`), so a better source (the system's
  disk counters, see Consequences) can replace the timing later without
  changing the callers.
- `PassEvent::GivingWay(bool)` and the keeper's matching event come from
  the disks' `DiskShare`s, so a stick has one state (giving way if any of
  its disks is: a stick with linked folders on two disks has two); the CLI
  and the app show it (rule 6).
- A drive ejected during a pause: the pause sleeps in slices of at most
  5 s and checks the drive's mark between them, as the pass does between
  parts, so `DriveMissing` is not a pause late.

## Consequences

- A big first share takes longer when the user works on the same disk, as
  much longer as they work. When they do not, nothing changes.
- The judgement is a guess from timing. It can mistake a slow stretch of
  the disk (bad sectors, a fragmented file, a drive spinning up) for
  another user, and give way for no reason; it recovers at the first fast
  window. It cannot see another user on a disk so fast that sharing it costs
  OurStick little (an SSD): there OurStick does not slow down, and the user
  does not notice it either.
- Measuring adds no I/O: only the time of reads OurStick does anyway.
- OurStick's own readers on one disk take turns (rule 5): a catalogue and
  a suitcase no longer read at once. On a spinning disk that is faster
  than today (no seeking between them); on an SSD, a little slower.
- Rejected: reading the system's disk counters (`/proc/diskstats` on
  Linux) to see others' I/O directly. Exact, but Linux only; macOS and
  Windows need IOKit or performance counter APIs, through `unsafe` or new
  dependencies. It can come later, behind a trait, if timing proves not
  enough; the `DiskLoad` trait is that place.
- Rejected: OS priority (Context). If Giorgos approves the `unsafe` calls
  later, they add to this; they do not replace it, since `mq-deadline` and
  the other non-BFQ schedulers ignore them.

## Order of work

1. `DiskShare` with tests: steady disk (never gives way), a busy stretch
   (gives way after two slow windows, back after one fast), one slow window
   alone (no change), small files and big ones mixed (no false alarm), a
   window of unread files (ignored), windows faster than any disk (fast,
   but no best), fading, the bounds of the pause.
2. `Timed` and the turns, with tests: windows inside one big file, the
   first read only (through a seek back), the queue's order, the suitcase
   for a waiting reader first but at most three to one, the pause outside
   the turn, a stick leaving the queue, the pause in slices with an
   ejected drive. If the turns grow past this, they get their own ADR.
3. The catalogue and the keeper use them; `LONGEST_YIELD` goes; the
   owner's words (CLI, app). An integration test: a pass over a folder whose
   reads are made slow (a `Read` that sleeps) says it gives way, and stops
   saying so when they are fast again.
4. Manual test on `sda`, with made-up files (not Rescued_Disk): a 20 GB folder of
   photos-sized files and one 10 GB file. Time copying a 5 GB file on the
   same disk alone, then while OurStick's catalogue runs, three times each,
   with the cache dropped before each run (`sync; echo 3 | sudo tee
   /proc/sys/vm/drop_caches`, run by Giorgos), and take the middle time. It
   passes if the copy takes at most 1.3 times as long as alone, OurStick
   keeps reading meanwhile (it does not stop), and it says "full speed"
   again within a minute of the copy ending. If not, the constants of rules
   2–4 are tuned with the logged numbers, and the ADR says so.
5. Docs: protocol.md (the owner's behaviour; nothing on the wire changes),
   gui-spec.md (the words), todo.md; status Accepted. Then Rescued_Disk.
