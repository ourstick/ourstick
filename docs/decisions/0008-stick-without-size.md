# ADR 0008: A stick without a fixed size, and folders shared as they are

**Status:** proposed — 2026-09-30. Changes the stick model of
`docs/gui-spec.md` ("The stick is a folder"). Builds on ADR 0007 and comes
after it.

## Context

The app's stick is a folder of a size the user picks at first launch (5, 10 or
20 GB), and dropped files are copied onto it. The size came from the USB stick
picture and from macOS, whose disk image is created with a size. Neither is a
real limit: a sparse image takes space only as it fills. With ADR 0007 the
owner no longer needs a second full copy either, so what a stick can hold is
set by the computer's disk, not by a choice.

The case that prompted this: a 913 GB folder on a disk with 491 GB free. Copied
onto a stick it does not fit anywhere; shared where it is, it takes no space.

## Decision

### 1. Capacity is what the computer has room for

The first launch has no size to choose. What can still be **copied** onto the
stick ("left") is:

```
left = free space of the disk that holds the stick's files
     - reserve  (10 % of that disk's total size, at most 20 GB)
     - the unused part of ADR 0007's working budget, only if the cache is on
       that disk (the part in use is already missing from the free space)
```

never below zero, recomputed when free space changes. For example, a 256 GB
laptop disk with 60 GB free and the cache on it: 60 − 20 − 6 = 34 GB left; a
30 GB disk with 10 GB free: 10 − 3 − 2 = 5 GB.

"The disk that holds the stick's files" is measured where the files really
are:

- **macOS:** the disk that holds the `.sparseimage` file, not the mounted
  volume (which reports the image's own size). The image is created sparse and
  as large as its disk, so it never limits first. It does not shrink when files
  are deleted until `hdiutil compact` runs while detached (a known limit today),
  so "left" can lag after deletions.
- **Windows:** the disk of the stick's folder (`%USERPROFILE%\OurStick\<name>`,
  ADR 0009), measured by that path, not the `subst` letter.
- **Linux:** the disk of the stick's folder (`~/OurStick/<name>`), which may
  differ from the data folder's.

The app shows one line, "2.1 GB used · 128 GB left", like a real stick; free
space is not shown on its own. Several sticks whose files are on one disk share
that disk's "left" (the reserve is counted once per disk): copying onto one
leaves less for the others. The watcher no longer enforces a size, so
`StickFull` from the watcher, `check_drive`'s size part and `drive_size` go
(the app finds its sticks by ADR 0009's `kind` instead).

### 2. What does not fit is refused, and can be shared as it is

A drop that does not fit in "left" is refused, as by a real USB stick: nothing
is copied, and the app says how big it is and how much is left. The same
message offers **Share it as it is**, for a folder or a single file: it is added
**linked**, shared from where it is, not copied, taking no space on the stick,
with no size limit of its own.

A stick is its own folder (copied files) plus zero or more linked folders or
files, each a top-level entry marked "linked". Rules:

- **Stored per stick** (a new table of linked paths); the manifest walk and the
  watcher take all of a stick's roots, not one.
- **No overlaps.** A linked path may not contain, or lie inside, the stick's own
  folder, the data folder, the cache, or another linked path. The app refuses
  such a drop and says why.
- **Links inside are not followed**, as today for the stick's own folder: a
  symbolic link in a linked folder is skipped, so it cannot bring in files from
  elsewhere.
- **Names:** a linked entry's name may not clash, ignoring letter case, with
  another top-level entry; on a clash the app adds " (2)", as it does for
  copies. If the linked folder itself is renamed or moved, it counts as gone.
- **Gone** (moved, deleted, its disk unplugged): like an ejected drive. The last
  published version stays offered; its files cannot be prepared, so readers
  waiting for them are told the sender is busy (ADR 0007: `NotReady`, never a
  failure), and the app says which linked entry is missing. Nothing new is
  published from it until it is back.
- **Files in constant use** (a virtual machine's disk, a browser profile): the
  watcher's quiet rule applies; a file that fails `encrypt_blob`'s check three
  times running is left out of the next versions and listed by the app as "in
  use, not shared" until it stays still.
- **Watching** 7,006 folders needs that many inotify watches on Linux; when the
  limit is reached, the watcher falls back to periodic scans, and the app says
  changes are seen more slowly.

The drive marker stays on the stick's own folder only; a linked folder is the
user's, never ours to mark.

### 3. Receiver

The receiver sees one stick. Before downloading, it says whether what is
chosen fits on the chosen disk ("needs 715 GB · fits"), and refuses to start
what does not fit. Choosing parts of a stick belongs to the receiver window,
decided on 2026-09-30 but outside v1 in `CLAUDE.md` until that is updated; the
CLI gets the fit check only.

## Consequences

- Migrations: `owned_sticks.drive_size` goes, after ADR 0009's `kind` has
  replaced it; a table of linked paths per stick comes. `docs/schema.md` and `docs/protocol.md` (the manifest walk over several
  roots) follow.
- `docs/gui-spec.md` and `docs/gui-mockup.html` change: the first launch without
  a size menu, the title without a size, the capacity line, the "does not fit"
  message with "Share it as it is", the linked mark, and the known limitation
  about the second copy (gone with ADR 0007).
- Tests: the capacity formula (cache on the same disk and on another; a small
  disk); a refused drop; overlapping linked paths; a linked folder that goes
  and comes back; a symbolic link inside a linked folder; name clashes
  differing only in letter case.

## Alternatives considered

- **Keep a chosen size.** Familiar, but it limits for no technical reason and
  makes the user guess.
- **A fixed share of free space (e.g. 75 %).** Leaves too much unused on big
  disks (123 GB of 491 GB).
- **Never copy, link everything.** Simplest on disk, but a dropped file would
  vanish from the stick when the user deletes or moves the original, unlike a
  real stick.
- **Ask on every drop whether to copy or link.** A question the user must learn
  to answer; refusing what does not fit asks only when it matters.
