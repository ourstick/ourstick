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

#[test]
fn a_big_file_moves_the_bar_while_it_comes() {
    use super::told::Landed;
    use ourstick_core::download::DownloadEvent;
    let mut landed = Landed::default();
    landed.on(&DownloadEvent::Planned { bytes: 2_100 });
    landed.on(&DownloadEvent::FileCopiedHere {
        path: "a",
        size: 100,
    });
    // An earlier run kept 1,500 of the film's 2,000.
    landed.on(&DownloadEvent::FileStarted {
        path: "film",
        size: 2_000,
        already_local: 1_500,
    });
    assert_eq!(landed.bytes(2_100), 1_600);
    landed.on(&DownloadEvent::Progress {
        path: "film",
        received: 300,
    });
    assert_eq!(landed.bytes(2_100), 1_900);
    // The encrypted blob is a little bigger than the file: never past it.
    landed.on(&DownloadEvent::Progress {
        path: "film",
        received: 700,
    });
    assert_eq!(landed.bytes(2_100), 2_100);
    landed.on(&DownloadEvent::FileReceived {
        path: "film",
        size: 2_000,
        already_local: 1_500,
        received: 700,
    });
    assert_eq!(landed.bytes(2_100), 2_100);
    // A new round starts again from nothing.
    landed.on(&DownloadEvent::Planned { bytes: 10 });
    assert_eq!(landed.bytes(10), 0);
}
