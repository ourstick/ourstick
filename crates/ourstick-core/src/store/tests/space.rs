//! The space ledger's claims, kept the same way by every [`Store`].

use super::*;

fn claim(seed: u8, kind: ClaimKind, disk: &str, bytes: u64) -> SpaceClaim {
    SpaceClaim {
        stick_id: owner(seed).stick_id(),
        kind,
        disk: disk.to_owned(),
        bytes,
    }
}

fn claims_are_decided_and_recorded_at_once(store: &dyn Store) {
    let cache = claim(1, ClaimKind::Cache, "dev:1", 10);
    let decided = store
        .claim_space(&mut |all| {
            assert!(all.is_empty());
            vec![cache.clone()]
        })
        .unwrap();
    assert_eq!(decided, std::slice::from_ref(&cache));
    // The decision sees what is there, and may record nothing.
    let none = store
        .claim_space(&mut |all| {
            assert_eq!(all, std::slice::from_ref(&cache));
            Vec::new()
        })
        .unwrap();
    assert!(none.is_empty());
    // One claim per stick and kind: a new one replaces it.
    let download = claim(1, ClaimKind::Download, "dev:2", 5);
    store.claim_space(&mut |_| vec![download.clone()]).unwrap();
    let bigger = claim(1, ClaimKind::Cache, "dev:1", 20);
    store.claim_space(&mut |_| vec![bigger.clone()]).unwrap();
    let mut all = store.space_claims().unwrap();
    all.sort_by_key(|claim| claim.kind == ClaimKind::Download);
    assert_eq!(all, [bigger, download.clone()]);

    store
        .update_space_claim(&download.stick_id, ClaimKind::Download, 2)
        .unwrap();
    store
        .remove_space_claim(&download.stick_id, ClaimKind::Cache)
        .unwrap();
    assert_eq!(
        store.space_claims().unwrap(),
        [SpaceClaim {
            bytes: 2,
            ..download.clone()
        }]
    );
    // Updating or removing what is not there is nothing.
    store
        .update_space_claim(&owner(9).stick_id(), ClaimKind::Cache, 1)
        .unwrap();
    store
        .remove_space_claim(&owner(9).stick_id(), ClaimKind::Cache)
        .unwrap();
    assert_eq!(store.space_claims().unwrap().len(), 1);
}

fn a_claim_must_name_its_disk_and_fit(store: &dyn Store) {
    for bad in [
        claim(1, ClaimKind::Cache, "", 1),
        claim(1, ClaimKind::Cache, "dev:1", u64::MAX),
    ] {
        assert!(
            store.claim_space(&mut |_| vec![bad.clone()]).is_err(),
            "{bad:?}"
        );
    }
    assert!(store.space_claims().unwrap().is_empty());
}

fn several_claims_are_recorded_together_or_not_at_all(store: &dyn Store) {
    let download = claim(1, ClaimKind::Download, "dev:2", 5);
    let scratch = claim(1, ClaimKind::Scratch, "dev:1", 2);
    let bad = claim(1, ClaimKind::Scratch, "", 2);
    assert!(store
        .claim_space(&mut |_| vec![download.clone(), bad.clone()])
        .is_err());
    assert!(store.space_claims().unwrap().is_empty());
    store
        .claim_space(&mut |_| vec![download.clone(), scratch.clone()])
        .unwrap();
    let mut all = store.space_claims().unwrap();
    all.sort_by_key(|claim| claim.kind == ClaimKind::Scratch);
    assert_eq!(all, [download, scratch]);
}

for_every_store!(
    claims_are_decided_and_recorded_at_once,
    a_claim_must_name_its_disk_and_fit,
    several_claims_are_recorded_together_or_not_at_all,
);
