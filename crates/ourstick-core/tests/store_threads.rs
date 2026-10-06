//! ADR 0018: an iroh-blobs store shut down leaves no thread behind. The
//! reader opens one per batch; with iroh-blobs 0.103 as published, each left
//! two or three for good (its runtime waited for itself to end). Its own
//! test binary, so no other test's stores are counted.

#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use iroh_blobs::store::fs::FsStore;

/// The threads of this process that iroh-blobs' stores run on.
fn store_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.starts_with("iroh-blob-store"))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn stores_shut_down_leave_no_threads() {
    let dir = tempfile::tempdir().unwrap();
    for batch in 0..20 {
        let store = FsStore::load(dir.path().join(format!("b{batch}")))
            .await
            .unwrap();
        store.add_bytes(vec![batch as u8; 100]).await.unwrap();
        store.shutdown().await.unwrap();
        drop(store);
    }
    // Their threads end on their own, soon after.
    let deadline = Instant::now() + Duration::from_secs(10);
    while store_threads() > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(store_threads(), 0);
}
