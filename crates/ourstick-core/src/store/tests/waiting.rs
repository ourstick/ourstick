//! Links the app waits to receive (ADR 0025): the same in every [`Store`].

use super::*;

fn waiting(seed: u8, added_at_ms: u64) -> WaitingLink {
    WaitingLink {
        stick_id: owner(seed).stick_id(),
        added_at_ms,
    }
}

fn waiting_links_are_kept_oldest_first_and_removed(store: &dyn Store) {
    assert_eq!(store.waiting_links().unwrap(), []);
    store.add_waiting_link(&waiting(2, 20)).unwrap();
    store.add_waiting_link(&waiting(1, 10)).unwrap();
    assert_eq!(
        store.waiting_links().unwrap(),
        [waiting(1, 10), waiting(2, 20)]
    );
    assert!(matches!(
        store.add_waiting_link(&waiting(1, 30)),
        Err(StoreError::StickExists)
    ));
    store.remove_waiting_link(&owner(1).stick_id()).unwrap();
    // Removing one that is not there is nothing.
    store.remove_waiting_link(&owner(1).stick_id()).unwrap();
    assert_eq!(store.waiting_links().unwrap(), [waiting(2, 20)]);
}

fn a_stick_waits_or_is_received_never_both(store: &dyn Store) {
    store.add_joined_stick(&joined(1, 10)).unwrap();
    assert!(matches!(
        store.add_waiting_link(&waiting(1, 20)),
        Err(StoreError::StickExists)
    ));
    // Received once it came: its wait ends in the same step.
    store.add_waiting_link(&waiting(2, 20)).unwrap();
    store.add_joined_stick(&joined(2, 30)).unwrap();
    assert_eq!(store.waiting_links().unwrap(), []);
    assert!(store.joined_stick(&owner(2).stick_id()).unwrap().is_some());
}

for_every_store!(
    waiting_links_are_kept_oldest_first_and_removed,
    a_stick_waits_or_is_received_never_both,
);
