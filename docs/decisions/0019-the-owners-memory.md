# ADR 0019: The owner's memory

**Status:** proposed — 2026-10-07. Reviewed by two other models against
our code; this version takes in their changes.

## Context

ADR 0017 made the owner's memory flat over time: it no longer grows while
it sends. It still grows with the stick. Measured with heaptrack
(2026-10-07): the owner of the 100 000-file stick, a fresh share (its
first catalogue) to a reader writing to RAM, grew from 46 MB to 219 MB of
RSS as it read the folder, its heap at most 176 MB; about 1.7 KB a file,
so about 1.7 GB of heap for a million files. ADR 0018 measured the same
shape on the reader and halved it. The peak, by where it was taken:

| What | MB |
|---|---|
| The files keeper loading a version (`files_keeper/version.rs`, `load_version`), all at once | ~93 |
| — the table of served files, a path each (`ServedFiles`), and the version's entries again (`Listed`) | ~50 |
| — every `owned_files` row of the stick (`Store::owned_files`) | ~20 |
| — the decoded manifest | ~16 |
| — the encrypted manifest read from the cache | ~8 |
| The keeper's state (`keeper.rs`, `learn`): a `ToMake` per blob, its paths and stamps | ~21 |
| The pass: the version being built and merged with the last (`partial.rs`) | ~20 |
| The made store's own table | ~4 |
| The rest (catalogue, SQLite, iroh) | ~38 |

Each file's path is held three or four times: in the pass's entries, the
keeper's decoded manifest, its `Listed` entries, the served table and its
`ToMake`s; and `load_version` holds all of them, and every row of the
store, at one moment.

## Decision

The keeper and the owner's store keep one decoded manifest of the version
they serve, the same the pass published, and refer to its entries by their
index. Nothing else changes: what is served, made or recorded, and when,
stays the same.

1. **One manifest, handed over.** When the pass publishes a version
   (`Versions::publish`), it hands its `Arc<Manifest>`, with the record's
   manifest hash, to the keeper (a `watch` channel beside `Demand`), and
   keeps nothing more of it once the pass ends. The keeper reads the
   cache file (`latest-manifest.blob`) only when sharing starts, before a
   pass has published, and then drops the encrypted blob and its
   plaintext as soon as they are decoded. So the manifest is decoded once
   per version, not again by the keeper two seconds later, and its
   version comes at once rather than at the keeper's next look.
2. **The served table by index, with its version.** `ServedFiles` holds the
   version's `Arc<Manifest>` and maps a blob's hash to the index of its
   first entry, with its plaintext hash; the path and size are read from
   the entry when a reader asks. What the store serves an export from is
   `(Arc<Manifest>, index)`: a failure is told only if the table still
   serves that blob from the same path (`old.entries[i].path` against the
   current table's entry), as today (ADR 0017's review fix), and the probe
   the same. `serve_more` carries the manifest it was made for, and is
   ignored for another (today the keeper alone keeps that true, by
   dropping makes of a replaced version). `share/served.rs` folds into
   the keeper's loading.
3. **The store's rows, walked, not loaded.** A new `Store::each_owned_file`
   hands each row of the stick to a closure (`FnMut(&OwnedFile) ->
   ControlFlow<()>`), in path order; each row is found among the entries
   by binary search (no assumption on SQLite's collation; the walk's order
   only keeps it near). What the keeper keeps of a row, by entry index, is
   its plaintext hash and stamp (about 60 bytes); the outboards are looked
   at on disk after the walk, never while it holds the store's one
   connection. `owned_files` stays for its other users.
4. **The keeper's state by index.** For every blob, its first index (for
   "a reader asks for this", and its place in the order of work); the
   files to make and the failed ones by index; and, for blobs at more than
   one path, their other indexes (a file's copies; at worst a few bytes a
   file). A `ToMake`, with its paths and stamps, is made from these when
   the blob is made, and `Made::Ready` carries the index it was made for.
   A changed blob's paths, to forget their records, come from the same.
5. **Not now: the pass's own entries** being built and merged
   (`partial.rs`, about 20 MB); measured after 1–4.

Expected, for 100 000 files: the manifest once (about 16 MB), the
keeper's and the table's maps (about 14 MB), the rows' plaintext hashes
while loading (about 6 MB, then gone), the transport's sets of allowed and
ready hashes (about 10 MB, unchanged); the owner's peak from 176 MB to
about 90 MB of heap, about 0.9 KB a file, most of the rest the pass and
iroh.

## Tests

The keeper's and the store's tests green with their values unchanged
(`keeper/tests.rs`, `made_store_tests.rs`, `tests/made.rs`,
`tests/not_ready.rs`); the served table by index: a blob at two paths, a
renamed file, a failure at an old version's path not taking out the
blob served at a new one, `serve_more` of an older manifest ignored;
`each_owned_file` on SQLite and the memory store, in byte order with
non-ASCII paths and `a/b` beside `a-b`, stopped early; `load_version`
with rows the manifest does not list and entries without a row; the
keeper given a version by the pass, without reading the cache file. A
gauge, as ADR 0018's: the keeper loading a version of 100 000 entries,
run by hand under heaptrack.

## Order of work

1. `Store::each_owned_file`, and `load_version` walking the rows (the
   outboards looked at after); still building today's types.
2. The manifest shared (from the pass, or the cache file at start), the
   served table and `serve_more` by index with their manifest, the
   store's exports and probe, and the keeper's state by index, in one
   commit: the table's type and the keeper's `ToMake`s change together.
3. Measured: the gauge, and the owner of the 100 000-file stick from a
   fresh share on Linux, macOS and Windows (its RSS at the end, beside
   219 MB here).

## Alternatives considered

- **Keep everything; a million files is rare.** 1.7 GB for a folder of a
  million photos is not rare enough, and the reader no longer needs that.
- **Keep the paths in SQLite only, and look them up when asked.** No
  memory, but a query for every blob a reader asks for, on the async path;
  the manifest in memory is 16 MB for 100 000 files and answers at once.
- **Share the pass's entries being built too.** About 20 MB more; they
  change on every part of the catalogue, so sharing them means copying on
  write or locking; rule 5 measures first.
