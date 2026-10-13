use super::*;
use std::sync::Arc;

fn state() -> AppState {
    AppState {
        data_dir: std::env::temp_dir(),
        store: Arc::new(ourstick_core::MemoryStore::new()),
        sticks: tokio::sync::Mutex::default(),
        disks: Arc::new(ourstick_core::disk::turns::DiskTurns::new(Arc::new(
            ourstick_core::disk::turns::SystemTimer::default(),
        ))),
        copying: std::sync::Mutex::default(),
        local_network_explained: Default::default(),
        keys_wait: Default::default(),
        started_moved: false,
        network_open: tokio::sync::watch::Sender::new(true),
    }
}

#[test]
fn the_answers_are_the_choices_offered() {
    assert_eq!(answer(None), Ok(None));
    assert_eq!(answer(Some("replace")), Ok(Some(OnConflict::Replace)));
    assert_eq!(answer(Some("keep_both")), Ok(Some(OnConflict::KeepBoth)));
    assert!(answer(Some("delete everything")).is_err());
    let copied = ourstick_core::drive::Copied { done: 3, total: 9 };
    let view = CopyView::from(CopyStep::Copying(copied));
    assert_eq!((view.measuring, view.done, view.total), (false, 3, 9));
    let view = CopyView::from(CopyStep::Measuring {
        files: 12,
        bytes: 400,
    });
    assert_eq!((view.measuring, view.files, view.total), (true, 12, 400));
}

#[test]
fn a_drop_can_be_stopped_while_it_runs_and_only_then() {
    let state = state();
    let id = ourstick_core::StickSigningKey::from_bytes(&[4; 32]).stick_id();
    let hex = crate::state::stick_hex(&id);
    let copying = Copying::start(&state, id).unwrap();
    // Another drop waits for this one.
    assert!(Copying::start(&state, id).is_err());
    assert!(!copying.stop.load(Ordering::Relaxed));
    stop_by(&state, &hex).unwrap();
    assert!(copying.stop.load(Ordering::Relaxed));
    drop(copying);
    // None runs: nothing to stop, and no error.
    stop_by(&state, &hex).unwrap();
    assert!(!is_copying(&state, &id));
}

#[test]
fn no_copy_onto_a_stick_being_changed() {
    let state = state();
    let id = ourstick_core::StickSigningKey::from_bytes(&[3; 32]).stick_id();
    tauri::async_runtime::block_on(async {
        crate::busy::make_busy(&state, &id, "Removing…")
            .await
            .unwrap();
        let refused = Copying::may_start(&state, id).await.err().unwrap();
        assert!(refused.contains("being changed"), "{refused}");
    });
    // Refused, it holds nothing: once the change is done, a copy may go.
    state.sticks.blocking_lock().remove(&id);
    assert!(tauri::async_runtime::block_on(Copying::may_start(&state, id)).is_ok());
}

#[test]
fn a_cut_copy_is_cleared_unless_a_drop_copies_now() {
    let state = state();
    let folder = std::env::temp_dir().join(format!("ourstick-cut-copy-{}", std::process::id()));
    let leftover = folder.join(".ourstick-copying-0");
    std::fs::create_dir_all(&leftover).unwrap();
    std::fs::write(folder.join(ourstick_core::manifest::DRIVE_MARKER), b"").unwrap();
    let id = ourstick_core::StickSigningKey::from_bytes(&[4; 32]).stick_id();
    let stick = ourstick_core::OwnedStick::new(
        id,
        "Photos",
        folder.to_str().unwrap(),
        ourstick_core::CipherId::None,
        0,
    );
    // A drop copies now: its hidden folder stays.
    let copying = Copying::start(&state, stick.stick_id).unwrap();
    clear_cut_copy(&state, &stick);
    assert!(leftover.exists());
    drop(copying);
    clear_cut_copy(&state, &stick);
    assert!(!leftover.exists());
    // The clearing holds no mark after.
    assert!(!is_copying(&state, &stick.stick_id));
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn one_copy_at_a_time_per_stick() {
    let state = state();
    let one = ourstick_core::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    let two = ourstick_core::StickSigningKey::from_bytes(&[2; 32]).stick_id();
    let copying = Copying::start(&state, one).unwrap();
    assert!(is_copying(&state, &one) && !is_copying(&state, &two));
    assert_eq!(
        Copying::start(&state, one).err().as_deref(),
        Some("Wait until the copy is done.")
    );
    // Another stick copies meanwhile.
    let other = Copying::start(&state, two).unwrap();
    drop(copying);
    drop(other);
    assert!(Copying::start(&state, one).is_ok());
}
