# Draft: iroh-blobs, let an `FsStore` run on a runtime it is given

Status: draft, not sent (2026-10-05). To be sent by Giorgos, as an issue
(and a PR if they welcome one) at https://github.com/n0-computer/iroh-blobs.
Checked against iroh-blobs 0.103.0.

---

**Title:** `FsStore`: option to run on a caller's tokio runtime instead of a
private one

**What we do.** Our app keeps a stick's encrypted blobs in short-lived
stores: a store per "suitcase" of a few thousand blobs, opened when readers
need them and removed whole when they are done (0.103 cannot delete single
blobs, so a whole store is our unit of eviction). An owner may open a few
dozen stores in a run, a reader one per batch it downloads.

**The problem.** `FsStore::load_with_opts` builds a new multi-thread runtime
for every store (`store/fs.rs`, `tokio::runtime::Builder::new_multi_thread()`
without `worker_threads`), so each store starts a worker thread per core and
stops them when it is dropped. On a 32-core Linux PC, serving 100 000 small
files, our owner ran about 290 threads. With glibc each new thread gets a
malloc arena, and arenas keep what is freed: RSS grew to 420 MB while the
heap in use stayed near 80 MB. We work around it today by setting
`TOKIO_WORKER_THREADS` before any runtime starts and by limiting glibc's
arenas, but the threads still come and go with every store.

**Proposal.** An option, e.g. `Options::runtime: Option<tokio::runtime::Handle>`
(or `FsStore::load_with_opts_on(handle, ...)`). With a handle:

- the store's actors and tasks are spawned on it, and no private runtime is
  built or dropped (no `RtWrapper`, no `block_in_place` on drop);
- the redb actor (`meta::Actor::run`), which makes blocking redb calls
  (`begin_write`, `commit`), runs on `spawn_blocking` or on a dedicated
  `std::thread` of its own, so it never blocks a worker of the caller's
  runtime;
- `shutdown()` ends the store's tasks and its database thread; the
  caller's runtime keeps running.

Without the option nothing changes. The default could stay a private
runtime (it isolates the store from a caller that blocks its workers),
with the trade-off written in the docs.

**Why it helps.** Many stores, one set of threads: no thread churn, and
memory that follows the work instead of the number of stores opened.

We would be glad to write the PR if the shape suits you.

---

Notes for us (not to send):
- The measurements behind the numbers: `docs/decisions/0007-rolling-blob-cache.md`
  (notes of 2026-10-04 and the withdrawn Amendment 1), `crates/ourstick-core/src/threads.rs`.
- If accepted: our suitcases (`transport/iroh/suitcases.rs`) and the reader's
  batches (`transport/iroh/batches.rs`) pass the program's runtime handle;
  `TOKIO_WORKER_THREADS` and maybe the arena limit can go.
