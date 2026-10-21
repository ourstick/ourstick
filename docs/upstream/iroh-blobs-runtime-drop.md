# Draft: an `FsStore` shut down leaves threads behind (iroh-blobs)

Status: a draft of an issue and a one-line pull request for iroh-blobs, not
sent yet. Found by OurStick (ADR 0018); patched meanwhile in
`third_party/iroh-blobs`.

## Issue

**Title:** `FsStore` leaves one or more runtime threads behind after shutdown

**Body:**

Every `FsStore` that is shut down and dropped leaves at least one
`iroh-blob-store-N` thread behind for the life of the process. A program
that opens and closes many stores (ours opens one per batch of a download)
accumulates them: 402 threads after 196 stores.

Reproduction (iroh-blobs 0.103.0 and 0.103.1, tokio 1.x, Linux):

```rust
#[tokio::test(flavor = "multi_thread")]
async fn stores_shut_down_leave_no_threads() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        let store = FsStore::load(dir.path().join(format!("s{i}"))).await.unwrap();
        store.add_bytes(vec![i as u8; 100]).await.unwrap();
        store.shutdown().await.unwrap();
        drop(store);
    }
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    let left = std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|t| std::fs::read_to_string(t.ok()?.path().join("comm")).ok())
        .filter(|name| name.starts_with("iroh-blob-store"))
        .count();
    assert_eq!(left, 0); // fails: 20
}
```

Cause, as far as we can see: the store's `Actor` owns the store's runtime
(`_rt: RtWrapper`) and runs on it (`handle.spawn(actor.run())` in
`load_with_opts`). When the last client goes, `run()` ends and the actor is
dropped on one of the runtime's own worker threads. `RtWrapper::drop` then
drops the runtime under `block_in_place`; the runtime's shutdown waits for
all of its blocking-pool threads to end, and the thread doing the drop is
one of them, so the wait never ends and that thread (and threads parked in
the half-shut pool) stay.

## Pull request

**Title:** fs store: shut the runtime down in the background on drop

```diff
 impl Drop for RtWrapper {
     fn drop(&mut self) {
         if let Some(rt) = self.0.take() {
             trace!("dropping tokio runtime");
-            tokio::task::block_in_place(|| {
-                drop(rt);
-            });
+            // Dropped on one of this runtime's own threads (the actor owning
+            // it ends there): waiting for every thread to end would wait for
+            // this one too.
+            rt.shutdown_background();
             trace!("dropped tokio runtime");
         }
     }
 }
```

With it, the reproduction above passes (0 threads left). Tasks still
running on the store's runtime are dropped rather than awaited, as
`shutdown_background` does; by then the actor, which owns everything the
store does, has already ended.
