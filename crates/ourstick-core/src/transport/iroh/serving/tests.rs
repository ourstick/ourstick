use std::time::Duration;

use iroh_blobs::store::mem::MemStore;

use super::*;

fn hash(byte: u8) -> Hash {
    Hash::from_bytes([byte; 32])
}

fn store() -> Store {
    (*MemStore::new()).clone()
}

fn serving_with(allowed: &[u8]) -> Serving {
    let serving = Serving::default();
    serving.allow(allowed.iter().map(|byte| hash(*byte)).collect());
    serving
}

#[tokio::test]
async fn only_allowed_plain_blobs_that_a_store_holds_ready_are_served() {
    let serving = serving_with(&[1, 2]);
    assert!(!serving.not_ready_within(Duration::from_secs(60)));
    assert!(matches!(serving.decide(hash(1), true), Decision::NotReady));
    assert!(serving.not_ready_within(Duration::from_secs(60)));
    serving.add_store(7, store());
    assert!(matches!(serving.decide(hash(1), true), Decision::NotReady));
    serving.set_ready(7, hash(1));
    assert!(matches!(serving.decide(hash(1), true), Decision::Serve(_)));
    // Not a plain blob, or not allowed: refused, ready or not.
    assert!(matches!(serving.decide(hash(1), false), Decision::Refuse));
    serving.set_ready(7, hash(3));
    assert!(matches!(serving.decide(hash(3), true), Decision::Refuse));
    // A new version that leaves it out.
    serving.allow([hash(2)].into());
    assert!(matches!(serving.decide(hash(1), true), Decision::Refuse));
}

#[tokio::test]
async fn a_store_leaves_only_once_no_transfer_holds_it() {
    let serving = serving_with(&[1]);
    serving.add_store(7, store());
    serving.set_ready(7, hash(1));
    let Decision::Serve(guard) = serving.decide(hash(1), true) else {
        panic!("not served");
    };
    let leaving = tokio::spawn({
        let serving = serving.clone();
        async move { serving.remove_store(7).await.is_some() }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!leaving.is_finished(), "left while a transfer held it");
    // Leaving: no new transfer starts from it.
    assert!(matches!(serving.decide(hash(1), true), Decision::NotReady));
    drop(guard);
    assert!(leaving.await.unwrap());
    assert!(matches!(serving.decide(hash(1), true), Decision::NotReady));
    assert!(serving.remove_store(7).await.is_none());
}

#[tokio::test]
async fn another_ready_store_serves_while_one_leaves() {
    let serving = serving_with(&[1]);
    serving.add_store(7, store());
    serving.add_store(8, store());
    serving.set_ready(7, hash(1));
    serving.set_ready(8, hash(1));
    assert!(serving.remove_store(7).await.is_some());
    assert!(matches!(serving.decide(hash(1), true), Decision::Serve(_)));
    assert!(serving.is_ready(8, &hash(1)));
    assert!(!serving.is_ready(7, &hash(1)));
}
