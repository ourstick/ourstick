# ADR 0007: Rolling blob cache ("suitcases")

**Status:** accepted — 2026-09-30. Its suitcases, the owner's budget, the
keeper and Amendment 2 are superseded by ADR 0017 (2026-10-05); the
catalogue, partial versions, readers' batches and "not ready" stay.
Supersedes the "Cache on disk" part of
ADR 0005 for the owner, and its single reader store; ADR 0005's rules on what
is served and its push refusal stay. The stick model that goes with it (no
fixed size, linked folders) is ADR 0008. Reviewed by two other models before
this version; checked against iroh-blobs 0.103's source.

## Notes from the implementation (step 4, 2026-09-30)

The plan for step 4 was reviewed again by two other models against the code
and iroh-blobs 0.103; these points changed on the way:

- **One decision point, in our handler.** The handler reads each request
  itself and decides under one lock whether it is served, not ready or
  refused, and registers the transfer with a guard on the suitcase it comes
  from. iroh-blobs' event hook decides nothing any more (it only counts bytes
  sent): with the store picked before the hook ran, a suitcase that changed
  in between would have answered `ERR_INTERNAL`, a failure.
- **Preparing reads each file once** (not "always `encrypt_blob` in full"):
  the nonce prefix comes from the catalogue's plaintext hash of that very
  blob, and the read checks it; a changed file's output is deleted at once
  and its record forgotten, so the next pass reads it again. ADR 0003 has the
  note. Without a recorded plaintext hash the file is read twice. So the
  owner reads a byte three times (two in the catalogue, one per suitcase), as
  the costs below say.
- **Blobs enter a suitcase by reference** (`ImportMode::TryReference`) to a
  file inside the suitcase's own folder, so it is written once; the file goes
  with the folder. (The alternative below rejected `TryReference` on a
  temporary file, which is removed while the store still points at it.)
- **Manifests are served from memory**, one small store per version, and the
  latest one is kept as `latest-manifest.blob` in the stick's cache folder
  for a restart; no store that is never emptied.
- **What readers hold:** a suitcase is never evicted while a reader is on it
  or on the one before it (read-ahead), and a reader that asked nothing for
  30 s holds none. A time-based rule (anything used in the last 30 s) would
  have stalled a fast reader on a small budget. A suitcase is at most 1 GB or
  a sixth of the budget, and at most 4 096 blobs (2026-10-04): each
  suitcase's store keeps the database pages it writes in memory (redb's
  cache, up to 1 GiB, not settable through iroh-blobs 0.103), and one
  suitcase of 100 000 small files held about 300 MB of them. Nothing is
  inlined in a suitcase's database either: a blob stays the file it is.
  Each suitcase's store also runs its own tokio runtime, a worker per core
  (32 on the test PC, about 290 threads in all): `threads.rs` sets
  `TOKIO_WORKER_THREADS` to 8 before any runtime starts, and the program's
  own runtime has at most 4 workers.
- **The keeper** (the policy) runs as a task per stick and learns from the
  transport which blobs readers ask for (`Demand`); a file bigger than the
  budget gets a suitcase alone once all others can go.

**Amended by ADR 0008's notes (step 6):** the budget is one per device, not
per program, split between the sticks shared at once through the space
ledger (`space_claims`); see there.

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

## Amendment 1: a suitcase no one is on sleeps (withdrawn, 2026-10-04)

**Withdrawn by its own step 0.** A test build shut the store of every
suitcase no reader held after 60 s unused. Threads stayed flat (15 → 20
instead of 15 → 48), but RSS grew as before or faster (237 → 366 MB over
19 000 files served, against 243 → 327 MB without it). The same build
without that change, with glibc's `MALLOC_ARENA_MAX=2`, stayed near flat
(196 → 222 MB over 30 000 files): what grows is glibc keeping freed memory
in its per-thread arenas, not the stores. That is addressed elsewhere; the
text below stays for the record.

Reviewed by two other models against the keeper, the iroh transport and
iroh-blobs 0.103's source; this version takes in their changes.

### Context

A suitcase leaves only when the budget needs its room (rule "What readers
hold" above). With the device's budget (42.9 GB on the test PC) that is
rarely: the suitcases a reader went past stay open for hours. Each open
suitcase is an iroh-blobs `FsStore`, and each `FsStore` runs its own tokio
runtime (8 worker threads since `threads.rs`) and its own redb database,
whose cache keeps the pages it wrote. Measured on the owner of 100 000 small
files (1.7 GB), one local reader, after the fixes of 2026-10-04 (4 096 blobs
per suitcase, nothing inlined, 8 workers per store): RSS grows about 20 MB
and 8 threads per suitcase the reader goes through, while the heap in use
stays near 80 MB. A stick of that size cut into 25 suitcases would hold
about 500 MB and 200 threads once one reader has it all, and keep them until
the program stops. Their disk space is what the budget is for; their memory
and threads are not.

### Decision

A suitcase is **open** (its store runs and serves), **asleep** (its store
is shut down; its folder, database and blob files stay on disk and in the
budget) or **gone** (evicted, as before). Invariant: a suitcase is asleep
only once its store has shut down; it is never asleep and waking at once.

0. **Measured first.** Before the code, the passed suitcases' stores are
   shut down by hand in a test build and RSS is checked to fall; if it does
   not, this amendment is withdrawn.
1. **Falling asleep.** The keeper gets a timer (a tick at the earliest
   `last_used + SLEEP_AFTER`). An open suitcase that no reader holds
   (`is_held`), that served nothing for `SLEEP_AFTER` (60 s, twice
   `IN_USE`), and that has no transfer in progress falls asleep:
   `Serving::retire_if_idle` takes its store out under the serving lock
   only if nothing is in flight, else does nothing and the keeper tries at
   the next tick. So sleeping never makes a request "not ready" while a
   transfer from it goes on, and never waits. A suitcase being prepared
   never sleeps. The `FsStore` itself is kept only by `Suitcases`, which
   shuts it down and keeps its folder (`asleep: id → folder`).
2. **Waking.** The keeper keeps each asleep suitcase's blob hashes (its
   `ready` set), whatever version it was cut from: `Serving` serves a blob
   from any store that holds it. A request for a blob that only an asleep
   suitcase holds gets `NotReady` (ADR 0007's "not ready yet, retry"), and
   the keeper wakes that suitcase — before it would prepare anything for
   that blob, and for the suitcase after the one a reader is on (read-ahead
   wakes too). Waking runs as its own task, not in the single preparing
   slot (a 1 GB preparation must not hold it up); a suitcase already waking
   is not woken twice. It opens the store again on the existing `blobs.db`,
   checks that the database still lists each blob it had and that each
   blob file is there with its size, and gives it back to `Serving`.
   A store that does not open, or lacks a blob, is evicted; a suitcase of
   the current version is then prepared again, and a blob of an older
   version is served by the current version's suitcases if it is still in
   it. Waking never serves less than preparing would.
3. **The budget does not change.** An asleep suitcase keeps its bytes in the
   budget (and the space ledger). When room is needed, asleep suitcases
   leave first (those of older versions first, then the oldest), before open
   ones that are not held; one that is asleep has only its folder to remove.
   "Never one a reader holds" stays.
4. **At most `AWAKE_MAX` (4) suitcases open per stick.** Held suitcases never
   sleep. When more than 4 are open, unheld ones fall asleep (as in rule 1,
   if nothing is in flight), used longest ago first, until 4 remain or none
   is unheld. The cap is per stick: a device sharing N sticks has up to 4N
   open, more while readers hold more.
5. **Stop and start.** Stopping a stick shuts its open stores and leaves the
   asleep folders, as today's suitcases; at the next start every suitcase
   folder of an earlier run is removed (`Suitcases::start`). Sleeping is for
   one run.
6. **The `Publisher` trait** gets `sleep(id)` (false if a transfer is in
   flight) and `wake(id)` (the blobs it serves again, or an error); `evict`
   covers asleep suitcases too. The in-memory publisher of the tests moves a
   suitcase's ready set to an asleep map and back.

### What it costs

- A reader that comes back to an asleep suitcase waits for one store to
  open (a database open and a runtime start, milliseconds to a second on a
  slow disk) instead of a whole preparation (reading and encrypting up to
  1 GB, longer while ADR 0013 gives way). A second reader of the stick an
  hour later is served from disk as before. Waking reads only the cache, so
  it takes no disk turn (ADR 0013).
- New code: the asleep state, the tick and the waking task in the keeper;
  `Suitcases` keeps asleep folders and opens them again; `Serving` takes a
  store out only when idle. The handler's single decision point is
  unchanged (an asleep suitcase is not in `Serving`, so its blobs are "not
  ready").

### Tests to write

- A suitcase no one holds falls asleep after `SLEEP_AFTER` (store shut
  down, folder there); a request for one of its blobs wakes it and is
  served, the blob identical.
- A suitcase with a transfer in progress (a reader on a slow big file) does
  not fall asleep; it does once the transfer ends.
- More than `AWAKE_MAX` open: the unheld ones used longest ago fall asleep;
  held ones never.
- Room needed: an asleep suitcase leaves before an open unheld one, one of
  an older version first.
- A new version while suitcases sleep: a blob it shares is woken, not
  prepared again.
- A request in the same tick as falling asleep, and two readers asking for
  the same asleep suitcase: each ends served, the store opened once.
- A store that does not open again (its database removed by hand): evicted,
  prepared again, the reader gets the blob; for an older version's
  suitcase, the blob comes from the current version.
- Stop while a suitcase is asleep or waking: nothing left running.
- Measured again on the 100 000-file test: memory and threads stay flat
  while the reader goes through the stick.

### Alternatives considered

- **Evict a suitcase no one is on after a while** (no asleep state). The
  least code, but a second reader, or the same one coming back, makes the
  owner read and encrypt it all again (on a hard disk, minutes; longer
  while ADR 0013 gives way): the cache would be no cache.
- **Only a cap on open suitcases.** Bounds memory, but suitcases above the
  cap would have to leave, as above.
- **One store per stick for every suitcase.** One runtime and one database,
  but iroh-blobs 0.103 cannot delete one blob (the reason suitcases exist).
- **Fewer threads per store (1 or 2).** Measured: 4 already made a download
  20–50 % slower; and the database caches stay.

## Amendment 2: a suitcase no one needs leaves (proposed 2026-10-05; superseded by ADR 0017)

Reviewed by two other models against the keeper and the transport; this
version takes in their changes (slow readers, pauses, the removal task).

### Context

Amendment 1 above was withdrawn by its own measurement, but what it was
after is real. A suitcase leaves only when the budget needs its room, so
with a budget of tens of GB every suitcase a reader went through stays open
until the program stops, each with its iroh-blobs store: its own tokio
runtime (8 threads, `threads.rs`), its redb database and its pool of entity
actors (up to 1 024, never trimmed). A review of our code found no map or
channel of ours that grows per blob served; the growth is per open
suitcase. Measured, owner of 100 000 small files with one local reader:

| | today | suitcases no one needs leave |
|---|---|---|
| macOS, whole stick | 139 → 206 MB, 40 → 208 threads | 110 → 121 MB, 40 → 66 threads |
| Linux, 19 000 files | 243 → 377 MB, 15 → 64 threads | 178–228 MB, 15 → 19 threads (with `mallopt`) |

(The Linux run also limited glibc's malloc arenas, see `threads.rs`:
glibc keeps what each thread's arena freed, which the closing alone does
not give back.) The macOS download took 564 s against 526 s, within the
noise of these runs.

### Decision

1. **Leaving when no one needs it.** An open suitcase is evicted, as for
   room (store shut down, folder removed, bytes back in the budget), when
   all of these hold:
   - no reader holds it (`is_held`, ADR 0016 rule 6, with the two changes
     of rule 2 below);
   - nothing is in flight from it;
   - it served nothing for `IDLE_LEAVE` (60 s, twice `IN_USE`), counted
     from the last transfer from it that *ended* (`last_used`, refreshed by
     rule 2), or from when it was prepared if none.
   A suitcase of an older version is never held (`is_held` looks at the
   current version's places), so it leaves once idle; while it still
   serves blobs the new version kept, its `last_used` keeps it.
2. **What holds a suitcase, made right for slow readers.** Today a reader
   holds nothing once it asked nothing for `IN_USE` (30 s): its asks are
   forgotten after that. Two changes:
   - **A transfer in flight holds its suitcase and the next** (read-ahead):
     a reader on one big file for minutes asks nothing, and the suitcase
     it will need next must not leave meanwhile. The keeper asks the
     transport, without waiting, whether a suitcase has transfers in flight
     (`Publisher::in_flight(id)`).
   - **A transfer that ends is told** (`Demand::Ended { connection, hash }`,
     sent when the transport drops the transfer's guard): the keeper
     refreshes that reader's `served` place and the suitcase's
     `last_used`, so a transfer longer than `IN_USE` does not leave the
     reader holding nothing at its end.
   - **A reader told "not ready" keeps its ask for `LONGEST_PAUSE + IN_USE`**
     (90 s), not `IN_USE`: its pauses grow to `LONGEST_PAUSE` (60 s), and
     the suitcase being prepared for it must not leave before it comes back.
3. **The idle check never waits.** The keeper looks every `IDLE_CHECK`
   (10 s, a steady interval in its `select!`, not reset by demand). It
   takes a suitcase out of serving with `Publisher::evict_if_idle(id)`, a
   plain (non-async) call that removes the store from serving under the
   serving lock only when nothing is in flight, and returns false otherwise
   (tried again at the next check). So a transfer is never cut, and a
   request after it gets "not ready" and the suitcase prepared again. The
   suitcase leaves the keeper's open list at once; shutting its store and
   removing its folder (`evict`) run in a task the keeper keeps in a
   `JoinSet`: polled in its `select!`, which gives the bytes back to the
   budget once the folder is gone, and awaited before the keeper returns
   (Stop, eject), so no store is left open. Between the two, the bytes are
   in neither the open list nor the free room: a preparation that needs
   them gets "no room" and retries every 500 ms, as today. (Eviction for
   room is unchanged: it runs in the preparing job and waits for transfers
   there, as before.)
4. **A dropped connection** forgets its reader at once, as today: its
   suitcases are then idle, and leave after `IDLE_LEAVE` if it does not come
   back. A reader that reconnects within that time finds them.
5. **Time in the keeper is injectable.** The keeper reads `Instant::now()`
   directly today; its state's decisions take the time as an argument (as
   `next_job` does), and the loop's tests run on tokio's paused clock, so
   the 60 s cases run at once.

Not in this amendment: the keeper's `Open.ready` and the transport's
ready set hold the same hashes (about 0.4 MB per suitcase); one of them
should go, in a change of its own (`docs/todo.md`).

### What it costs

- A suitcase asked for again after it left is read and encrypted again
  (per suitcase: up to 4 096 files or 1 GB; for small files on the test
  PC, tens of seconds). That happens to a second reader, or to one that
  comes back after more than `IDLE_LEAVE` of pause. The "asleep" suitcase
  of Amendment 1 would avoid it at the cost of a third state; it can come
  later if second readers turn out to be common.
- New code: the check and the removal tasks in the keeper's loop;
  `evict_if_idle` and `in_flight` in the `Publisher` trait (iroh, and the
  tests' in-memory publisher, which then counts transfers in flight per
  suitcase); `Demand::Ended` from the handler (and the in-memory fetcher);
  asks kept longer after "not ready".

### Tests to write

- State, with the time given: a suitcase no one holds, unused for
  `IDLE_LEAVE`, is a candidate; one held, or with a transfer in flight, or
  fresh, is not; the one after a suitcase with a transfer in flight is
  not.
- Serving: `evict_if_idle` is false while a transfer's guard lives, true
  after it drops; a request then gets "not ready".
- Keeper loop, paused clock: an idle suitcase is evicted, its folder gone
  and its bytes back; asked for again, it is prepared once and served, the
  blob identical, with no loop of evictions.
- A transfer longer than `IN_USE` from suitcase n: n + 1 is still there
  when it ends.
- A reader in "not ready" pauses of 60 s: the suitcase prepared for it is
  there when it comes back.
- A reader that reconnects within `IDLE_LEAVE`: its suitcases are there;
  after it: prepared again.
- A new version while suitcases are idle: blobs it kept are served, the
  old suitcases leave once idle.
- Stop while a removal task runs: the store closed, the folder gone.
- Measured again on the 100 000-file test, Linux and macOS: memory and
  threads flat while the reader goes through the stick.
