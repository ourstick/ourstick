# ADR 0007: Rolling blob cache ("suitcases")

**Status:** proposed — 2026-09-30. Supersedes the "Cache on disk" part of
ADR 0005 for the owner, and its single reader store; ADR 0005's rules on what
is served and its push refusal stay. The stick model that goes with it (no
fixed size, linked folders) is ADR 0008. Reviewed by two other models before
this version; checked against iroh-blobs 0.103's source.

## Context

A real case: a rescued disk to send to a relative, 913 GB in 146,297 files and
7,006 folders, on a hard disk that reads 126 MB/s. The owner's computer has
491 GB free on that disk and 167 GB on the system disk. Today this cannot be
shared at all:

- **Owner, ADR 0005:** every file is encrypted into a full second copy, the
  cache. Worse, `publish_folder` encrypts the whole stick into `staging/` first
  and only then imports each blob with `ImportMode::Copy`, so preparing takes up
  to twice the stick's size, all in the data folder on the system disk.
- **Reader:** its one blob store keeps every downloaded blob until the whole
  download ends, so it too needs about twice the stick's size.

What is not the problem: CPU and memory. Measured on the owner's PC (2 GiB,
real code): preparing about 195 MB/s and downloading about 140 MB/s, each on
about one core, 38 MB of memory for both. The network decides the time: at
100 Mbps 913 GB take about 21 hours, at 30 Mbps 2.8 days, at 10 Mbps (or relay
only) 8.5 days. Encryption does not change these numbers, so it stays.

What iroh-blobs 0.103 allows (checked in its source): a store serves only blobs
it holds; deleting a blob is not public (`blobs().delete` is `pub(crate)`,
"users should rely only on garbage collection"), and its GC stops for good
after one error (`docs/protocol.md`, "Known limits"); `ImportMode::TryReference`
keeps a reference to the source file, which must then stay. Its provider is
public in pieces, though: `StreamPair::accept`, `read_request` and
`handle_get(pair, store, request)` let a handler of ours pick the store for
each request.

## The invariant

A blob leaves the owner only if it is in a store, and it enters a store only if
its BLAKE3 hash equals the hash in the current manifest, which came from a
complete `encrypt_blob` run. Equal hash means identical bytes, so the same
nonce never carries two plaintexts (ADR 0003). This holds for every mechanism
below; it also covers a file swapped for another (e.g. by a link) between two
reads, which ADR 0003's `PlaintextChanged` check alone would not see.

## Decision

### 1. Catalogue first, without storing ciphertext

The owner reads every file to build the manifest: the plaintext hash (the nonce
prefix, ADR 0003), then an encryption pass that only computes the blob's hash
and throws the bytes away. Nothing is written but the per-file record in
`owned_files`, now with the plaintext hash too; that hash is a hint for
progress only, and every later encryption runs `encrypt_blob` in full. Files are
catalogued a few at a time in parallel; a restart skips files whose stamp is
unchanged, whether or not their blob is in a store.

A big catalogue takes hours (913 GB on that disk: about 4 h, two reads per
file), so it is **published as it grows**: every few minutes the publisher signs
a new version (a real `seq`) with the files catalogued so far, marked
**partial**; the last one is marked complete. The mark is a field of the
manifest, which is why the manifest format version goes up, once, to 2, with
the stick's name too (ADR 0009; empty until that ships): a reader too old to
know them refuses the new manifest with a clear message instead of reporting
half a stick as done.

There is **one publisher task per stick**: the catalogue, watcher events and
the republishing of a changed file are one job, and changes seen during the
catalogue go into its next version.

### 2. Suitcases: stores that come and go

Blobs are served from **suitcases**: runs of blobs in manifest order, up to a
size (a few GB), each in **its own `FsStore` in its own folder**. Preparing one
runs `encrypt_blob` on each file again and imports the result with
`ImportMode::Copy` (or `add_stream`); the result is kept only if its hash equals
the manifest's. A mismatch means the file changed: it is catalogued again and
published in the next version, and until then its old hash is not ready. The
wrong bytes sit unserved in that suitcase's folder until it goes.

- **Serving.** The blobs protocol is our own handler: it accepts each stream
  (`StreamPair::accept`), reads the request, and passes it to `handle_get` with
  the store of the suitcase that holds the hash. The event handler that
  decides every request (ADR 0005) stays; it now answers three ways:
  - **allowed and in a ready suitcase:** served;
  - **allowed but not ready** (no suitcase yet, being prepared, or being
    evicted): `AbortReason::RateLimited`, sent as `ERR_LIMIT`, which resets that
    stream only; the suitcase is queued;
  - **not allowed:** `Permission`, as today.
- **Read-ahead.** While a reader is on one suitcase, the next one is prepared,
  so a single reader rarely waits.
- **In flight.** A blob is in flight from the accepted `GetRequestReceived`
  until its `Completed`, `Aborted`, or its connection's `ConnectionClosed`. To
  evict, a suitcase is first marked **evicting** (new requests for it get
  `RateLimited`), then closed and its folder removed once nothing in it is in
  flight. Removing a folder is the only way blobs leave; no GC is involved.
- **Budget.** One working budget per program, shared by its sticks: 10 % of
  the free space on the cache's disk, at least 2 GB, at most 40 GB, reserved
  whole (ADR 0008 counts it that way). Suitcases are prepared first come, first
  served, per stick in turn, and each reader has at most one suitcase prepared
  ahead, so neither a jumping reader nor a busy stick holds the whole budget.
  When it is full, the next suitcase waits and its readers get `RateLimited`.
  With less than 2 GB free, big sharing does not start, and the app says why.
- **Restart.** When a stick starts sharing, after its lock is taken (ADR 0009),
  its own cache folder (`cache/<hex stick id>/`) is removed before any of its
  stores is opened; suitcases are prepared again as they are asked for.
  Another program's sticks, and their folders, are never touched. No temp tags are
  re-adopted (upstream #247), and nothing depends on GC.
- **Where.** The cache lives on the disk the user chooses (default: the data
  folder).

### 3. Reader: batches that come and go, and waiting

The reader mirrors the owner: it downloads the manifest's entries in batches
(a few GB, in manifest order), each batch into **its own store folder**. When
every entry of a batch is verified and decrypted into place, the batch's folder
is removed. An entry whose hash appears again later (identical files, ADR 0003)
is copied locally from the file already in place, never fetched twice. Resume
works as today inside a batch, and finished batches are known from
`downloaded_files`. At its peak the reader needs the size of what it chose,
plus one batch, plus the largest file (its decrypted copy before it moves into
place).

`ERR_LIMIT` becomes its own transport error, `NotReady`, which is not a
failure. The reader backs off (up to a minute) and asks again, and says the
sender is preparing. Every few minutes while it waits, it fetches the record
again and follows a newer version. It never gives up on `NotReady` alone: a
sender that stays busy looks like a sender that is offline, and the user can
stop. A **partial** manifest does not end a download: the reader waits for
newer versions until one is complete (versions of a growing catalogue are not
counted against `MAX_NEW_VERSIONS`, which stays for refusals).

### 4. Several sticks

ADR 0009: each stick has its own node key and endpoint, so the suitcase stores
and the handler of section 2 are per stick. The budget is one per program and
shared by its sticks (first come, first served, per stick in turn), and so is
the cache folder, one subfolder per stick.

## Order of work

1. Reader batches and the per-batch store removal (useful on their own: the
   reader stops needing twice the space).
2. `NotReady` in the fetcher, and manifest format 2 (the partial mark and the
   stick's name field).
3. Owner: the catalogue as its own pass, published as it grows.
4. Owner: suitcase stores, our blobs handler, in-flight tracking, budget.
5. ADR 0009's store part: `kind`, `node_id`, the node key per new stick, the
   locks.
6. ADR 0008 (stick without size, linked folders).
7. The rest of ADR 0009: moving the first stick, several sticks in the app,
   the tray, the receiver list.

Each step ends with `cargo test` green, and `docs/protocol.md`, `docs/schema.md`
(the plaintext hash in `owned_files`) and the CLI's cache estimate updated.

## Tests to write

A two-peer transfer through several suitcases; `NotReady` then served; eviction
while a transfer is in flight; a file changed between catalogue and suitcase; a
catalogue followed through more than 10 partial versions; restart with a
non-empty cache folder; two programs sharing different sticks, each
restarting without touching the other's cache; identical files across batches; the budget with the
cache on the stick's disk and on another; two readers at different places
sharing a small budget.

## Costs

- **Three reads per byte on the owner:** two while cataloguing, one per
  suitcase preparation (another for each later reader who needs an evicted
  one). On a hard disk the catalogue sets when the first complete version is
  out; preparing runs alongside the transfer.
- **Our own blobs handler:** small, but it sits between iroh-blobs' public
  pieces, so iroh-blobs upgrades must re-check it.
- **Code that changes, not only grows:** `publish` (today it needs every
  allowed blob present), `known_files` (today reuses a stamp only if the blob
  is in the store), `publish_folder` and the watcher's publish job, the event
  handler, the fetcher's error mapping, the reader's store handling, and the
  CLI's cache estimate before `share`.
- **Wire change:** `ERR_LIMIT` means "not ready yet, retry", and the manifest
  format version goes up to 2 for the partial mark and the name.

## Alternatives considered

- **Keep ADR 0005's full cache.** A 913 GB stick needs 913 GB more.
- **One store, eviction through GC** (`add_protected`). Deletion only on the GC
  interval, and GC stops for good after one error.
- **Delete blobs one by one.** Not public in iroh-blobs 0.103.
- **`ImportMode::TryReference` on a temporary file.** The store keeps pointing
  at the file; removing it corrupts the blob.
- **One endpoint for every stick.** Replaced by a node per stick (ADR 0009):
  one node ID for all sticks tells a reader of two that they share an owner.
- **Encrypt on the fly with our own provider** (outboards only, about 0.4 %).
  Smallest on disk, but replaces iroh-blobs' verified streaming and needs the
  check per chunk. Kept for later if the budget proves too large.
- **Stopping the reader after N minutes of `NotReady`.** A full budget or a
  queue is normal and can last longer; the reader would fail for nothing.
- **The user splits the folder into several sticks by hand.** More links and
  bookkeeping, no time gained.
- **Optional encryption for big sticks.** No time gained (the network
  decides), and the public part of a link, which passes through the
  ourstick.com page and link previews, would be enough to read everything.
