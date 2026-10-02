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

1. **OurStick measures its own reading, and slows down when it gets
   slower.** Each part of the catalogue (at most 256 files or 256 MB) and
   each file put in a suitcase reports how many files and bytes it read and
   how long it took. A part far slower than this disk has recently been for
   OurStick means someone else is using the disk. Then OurStick pauses after
   each part, so that it reads only a small share of the time; when parts
   become fast again, it goes back to full speed. No OS priority, no
   `unsafe`, the same on every system.
2. **Speed is judged against similar work.** Small files cost a seek each,
   whatever their size; big ones cost their bytes. A part is judged by what
   it should have taken at this disk's recent best: a cost per file and a
   cost per byte, each the best of the recent parts of that kind (parts of
   mostly big files teach the cost per byte, parts of many small ones the
   cost per file). The recent best fades slowly (about by half over an
   hour of parts), so a disk whose inner tracks are slower, or a cache that
   made the first parts look fast, does not count as "someone else" for
   long.
3. **Someone else is there** when two parts in a row took more than three
   times what they should have. Three, not two: the inner tracks of a disk
   are up to half as fast as its outer ones, and a second sequential reader
   on a spinning disk costs far more than that. **They are gone** when one
   part takes less than one and a half times what it should have.
4. **Giving way:** while someone else is there, after each part OurStick
   pauses nine times as long as the part took (it reads a tenth of the
   time), at least 1 s, at most 60 s. Parts stay small (as today), so the
   user's program waits at most one part for the disk.
5. **Readers waiting still matter, but do not take the disk back from the
   user.** The catalogue's pause for waiting readers (today's
   `LONGEST_YIELD`) stays; a suitcase for a waiting reader gives way to the
   user as well. A reader who waits longer because the owner's disk is busy
   is told nothing new: "The sender is getting it ready" is already what
   they see.
6. **The owner sees it.** While giving way, a stick's state says so:
   "Reading the folder slowly while another program uses the disk." No
   setting: a stick that never gives way is not something a user asks for,
   and one that always crawls is what this avoids.
7. **Not the reader's side, not now.** A receiver writes at the network's
   speed, which is far below a disk's except on a fast home network; its
   first round compares files already in the folder by content, which is
   rare and short. Both stay as they are, and the todo says so.

## In the code

- `share/disk_share.rs` (new, pure, no I/O): `DiskShare`, with
  `after_part(files, bytes, took) -> Duration` (the pause before the next
  part) and `giving_way() -> bool`. The recent bests and their fading are
  in it; time comes in as `took` and as the caller's `Instant`s, so tests
  drive it with numbers only.
- `share/pass.rs`: after each part, `DiskShare::after_part` (the bytes
  read: the catalogue's `read` files, twice their size); the pause is the
  larger of it and today's yield to waiting readers.
- `share/keeper/prepare.rs`: one `DiskShare` per stick's keeper, fed after
  each file it encrypts into a suitcase. The pass and the keeper of a
  stick read the same disk at the same time; each judges its own parts.
  Sticks on different disks do not slow each other; sticks on one disk each
  see the other as "someone else" only if their parts get slower, which
  makes both give way, then both speed up: a slow sway, not a stall, and
  rare (a pass and a suitcase of one stick are already paced by rule 5).
- An event `PassEvent::GivingWay(bool)` (and the keeper's equivalent) for
  the owner's state words, rule 6.

## Consequences

- A big first share takes longer when the user works on the same disk, as
  much longer as they work. When they do not, nothing changes.
- The judgement is a guess from timing. It can mistake a slow stretch of
  the disk (bad sectors, a fragmented file, a drive spinning up) for
  another user, and give way for no reason; it recovers at the first fast
  part. It cannot see another user on a disk so fast that sharing it costs
  OurStick little (an SSD): there OurStick does not slow down, and the user
  does not notice it either.
- Measuring adds no I/O: only the time of reads OurStick does anyway.
- Rejected: reading the system's disk counters (`/proc/diskstats` on
  Linux) to see others' I/O directly. Exact, but Linux only; macOS and
  Windows need IOKit or performance counter APIs, through `unsafe` or new
  dependencies. It can come later, behind a trait, if timing proves not
  enough.
- Rejected: OS priority (Context). If Giorgos approves the `unsafe` calls
  later, they add to this; they do not replace it, since `mq-deadline` and
  the other non-BFQ schedulers ignore them.

## Order of work

1. `DiskShare` with tests: steady disk (never gives way), a busy stretch
   (gives way after two slow parts, back after one fast), small files and
   big ones mixed (no false alarm), fading (an early cached burst stops
   counting), the bounds of the pause.
2. The pass and the keeper use it; the owner's state words (CLI and app).
3. Manual test on `sda`: share a large test folder (not Rescued_Disk: made up
   files, about 20 GB), and while its catalogue runs copy a 5 GB file on
   the same disk; time the copy with and without OurStick running, and see
   OurStick's state say it is giving way and then stop. The copy should
   take at most about 1.3 times as long as alone.
4. Docs: protocol.md (the owner's behaviour; nothing on the wire changes),
   gui-spec.md (the words), todo.md; status Accepted. Then Rescued_Disk.
