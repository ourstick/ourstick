use super::*;
use ourstick_core::locks::download_lock;

fn stick(seed: u8) -> StickId {
    ourstick_core::StickSigningKey::from_bytes(&[seed; 32]).stick_id()
}

#[tokio::test]
async fn a_stick_is_held_with_its_lock_until_it_is_let_go() {
    let data = crate::testing::TempFolder::new();
    let receivers = Receivers::default();
    let lock = download_lock(data.path(), &stick(1)).unwrap();
    receivers.hold(stick(1), Some(lock)).await;
    assert!(receivers.is_held(&stick(1)).await);
    // Held: no other program takes it meanwhile.
    assert!(download_lock(data.path(), &stick(1)).is_err());
    let lock = receivers.release(&stick(1)).await;
    assert!(lock.is_some());
    drop(lock);
    assert!(!receivers.is_held(&stick(1)).await);
    assert!(download_lock(data.path(), &stick(1)).is_ok());
    // Letting go of one not held is harmless.
    assert!(receivers.release(&stick(2)).await.is_none());
    // Held without its lock (another program had it): let go all the same.
    receivers.hold(stick(3), None).await;
    assert!(receivers.release(&stick(3)).await.is_none());
    assert!(!receivers.is_held(&stick(3)).await);
}

#[test]
fn the_quit_dialog_hears_which_sticks_are_downloading() {
    let receivers = Receivers::default();
    receivers.show(stick(1), |shown| shown.state = "downloading");
    receivers.show(stick(2), |shown| shown.state = "up_to_date");
    receivers.show(stick(3), |shown| shown.state = "getting_more");
    let mut downloading = receivers.downloading();
    downloading.sort_by_key(|id| id.to_bytes());
    let mut expected = vec![stick(1), stick(3)];
    expected.sort_by_key(|id| id.to_bytes());
    assert_eq!(downloading, expected);
    assert_eq!(receivers.shown(&stick(9)).state, "");
}
