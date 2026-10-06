# ADR 0018: The reader's memory

**Status:** proposed — 2026-10-06. Reviewed by two other models against our
code and iroh-blobs 0.103.0; this version takes in their changes.

## Context

ADR 0017 made the owner's memory flat. The reader of the same 100 000-file
stick still reaches 206 MB on macOS, 280 MB on Linux (with two glibc arenas;
490 MB before), and 111 MB at 70 % on Windows; and it grows with the stick.
Measured on Linux with heaptrack (2026-10-06, a release build with symbols,
the reader writing to RAM; 100 000 small files in 7-level folders, about two
folders per file, as in the big folder), the peak is 196 MB of heap, nearly
all of it the plan (ADR 0010), made whole before the first file is asked for:

| What, at the peak | MB | Held as |
|---|---|---|
| Where each file is saved, and its folders | ~63 | `saved` (a full path per file, also when unchanged), a copy of it for the renamed lines (`placed`), `folders` (stick folder → saved folder, two full paths per folder) |
| The names already taken | ~55 | `taken.files` (one folded path per file), `taken.folders` (two full paths per folder) |
| Which places are links | ~41 | `Looked.links`: a `PathBuf` per folder and per file component (`folder/disk.rs`) |
| The manifest | ~12 | `Entry` per file; and the whole manifest cloned again in `joined.rs` and `listing.rs` |
| The download's rows | ~11 | `DownloadedFile`, two strings per recorded file |
| The rest (transfer, iroh, SQLite) | ~14 | |

About 1.7 KB a file, nearly all of it strings: each folder is held as its
full path four times over, each file twice or three times.

The reader's threads grew from 15 to **402** in the same run. Each batch
(ADR 0007: 512 files or 1 GB) is an iroh-blobs `FsStore` of its own; each
closed store leaves threads behind for good (48 after 10 000 files, 20
batches). The cause is a deadlock in iroh-blobs 0.103.0 and 0.103.1: the
store's actor owns the store's tokio runtime and runs on it; when the last
`FsStore` clone goes, the actor ends and drops the runtime on one of the
runtime's own threads (`RtWrapper::drop`, `block_in_place(|| drop(rt))`,
`store/fs.rs:838`); the runtime's shutdown waits for every one of its
threads to end, that one included, so it waits forever. Not our code: we
hold one `FsStore` per batch and drop it before the next. The fix upstream
is one line: `rt.shutdown_background()` in place of the `block_in_place`
drop. Threads cost little memory each with two arenas, but they grow
without end: a million files would mean about 4 000, past some systems'
limits.

## Decision

### Part A: the same plan, held small

A1. **Compact the plan; ADR 0010's choices stay exactly as they are.**
- **No second manifest:** `joined.rs` and `listing.rs` borrow it, not clone
  it (~12 MB).
- **Folders as a tree:** one node per folder of the stick (its name, its
  parent, the name it is saved under, its folded name), shared by what is
  now `folders` and `taken.folders`. The taken set holds (parent node,
  folded name, kind, holder) instead of folded full paths: the same
  collisions, since folding (case, and Unicode NFC on macOS) never crosses
  a `/`. A test with Greek names in NFD and final sigma, and the existing
  macOS and Windows ones, check it.
- **A file's saved name only when its own name changed:** under a renamed
  folder (`Trip_/`) every full path differs, but nearly no file's own name
  does; the full path is made from the tree when needed.
- **`Looked.links` by folder node only**, not one `PathBuf` per component of
  every file.
- **No copies for the renamed lines** (`placed`), and lookups by the
  manifest's own order (it is sorted, `encoding.rs`) in place of the
  `by_path` and `index_of` maps.

Expected: the plan of the 100 000-file stick from about 170 MB to under
40 MB, the reader's peak from 196 MB to about 60 MB of heap. Guarded by ADR
0010's tests unchanged (`saved_paths/tests.rs`, `saved_paths/edge_tests.rs`,
`folder/tests.rs`, `folder/chosen_tests.rs`, the scripts test), plus a new
test that plans 100 000 files in deep folders under a stated heap budget.
The app's receiver uses the same plan (`apps/app/src-tauri/src/receiving/`)
and is measured too; so are `preview` and `list`.

A2. **Fetch while the plan is made.** Only if A1 leaves the time to the
first file a problem. Recorded and unchanged places are found for the
whole stick first (rule 4: a renamed `a?.jpg` must not take `a_.jpg` before
a real `a_.jpg` later in the stick is seen), cheap with A1's taken set; only
the renamed rest is chosen batch by batch. The room check stays whole: it
needs the bytes to write (each file's look on disk, ADR 0008), not the
manifest's sum. Its own review before it is built.

### Part B: no threads left behind

B1. **Report it upstream, with the fix** (`docs/upstream/`): the deadlock,
a reproduction (open and shut down 50 `FsStore`s, count the threads left),
and the one-line change. Beside the shared-runtime draft, which would also
avoid it.

B2. **Until it is released, one of:**
- **B2a, the fix in our build:** iroh-blobs 0.103.1 with that one line, as
  a `[patch.crates-io]` of a copy in `third_party/` (the root `Cargo.toml`
  changes, so it is Giorgos's call). Removes the leak outright; the copy
  goes when upstream releases the fix. Tested by a test of ours that opens
  and shuts down stores and counts threads.
- **B2b, far fewer stores:** a batch closes at 1 GB or at 4 096 files, not
  512: 25 stores for the 100 000-file stick, about 60 threads left instead of
  400. Divides the leak by eight, does not end it. A batch's small blobs (up
  to 16 KiB) live in its store's database, so an open batch may hold up to
  64 MB there, eight times today's: measured. Batches are named by how they
  are cut, so a download stopped before the change starts its current batch
  again (files already placed stay). `docs/protocol.md` and ADR 0007 are
  updated.

B3. **Not now: one store for a whole download**, blobs removed by tags and
iroh-blobs' GC: GC deletes untagged partial blobs and stops for good after
an error (our report upstream, `docs/todo.md`).

## Order of work

1. B1's report written; B2a or B2b (Giorgos chooses), with its test,
   measured: threads and memory over 100 000 files.
2. A1 in small commits, each with ADR 0010's tests green: the manifest not
   cloned; the folder tree for `folders` and `taken`; saved names only when
   changed; links by folder; no copies for renamed lines. Measured after the
   last on Linux, macOS and Windows, the app's receiver too.
3. A2 only if step 2's numbers ask for it, after its own review.

## Alternatives considered

- **Keep everything, rely on two arenas.** Linux is fine at 280 MB, but the
  plan still grows with every file, macOS and Windows have no arena setting,
  and threads still grow.
- **Store the plan in SQLite.** No memory, but every name check becomes a
  query, and ADR 0010's rules need many per file; slower for every stick to
  save memory in big ones. A1's tree is small enough.
- **Bigger batches by bytes too** (4 GB). Fewer stores again, but the reader
  then needs twice that free while it runs (ADR 0008's scratch room).
- **A compact manifest** (fixed records and one buffer of names): about
  5 MB less, and every user of `Manifest` changes; not worth it.
