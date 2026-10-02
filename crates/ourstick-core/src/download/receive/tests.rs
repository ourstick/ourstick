use super::*;
use crate::record::RecordError;
use crate::space::SpaceError;

fn io(kind: std::io::ErrorKind) -> DownloadError {
    DownloadError::Io {
        path: "/d/Holiday".into(),
        source: std::io::Error::from(kind),
    }
}

#[test]
fn a_sender_that_is_off_is_waited_for_longer_each_time_up_to_five_minutes() {
    let off = DownloadError::Transport(TransportError::Unreachable("no route".to_owned()));
    let mut waited = Duration::ZERO;
    let mut pauses = Vec::new();
    for _ in 0..8 {
        match after_error(&off, None, waited) {
            Next::Wait(Waiting::Sender(reason), after) => {
                assert_eq!(reason, "no route");
                assert_eq!(
                    after_error(
                        &DownloadError::Transport(TransportError::Unreachable(
                            "\u{1b}[2J".to_owned()
                        )),
                        None,
                        Duration::ZERO
                    ),
                    Next::Wait(Waiting::Sender("\\u{1b}[2J".to_owned()), FIRST_WAIT)
                );
                pauses.push(after);
                waited = after;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(pauses[0], FIRST_WAIT);
    assert!(pauses.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(*pauses.last().unwrap(), LONGEST_WAIT);
}

#[test]
fn what_the_owner_or_this_computer_can_fix_is_waited_for() {
    let cases = [
        (
            DownloadError::Transport(TransportError::UnknownStick),
            Waiting::NoLongerShared,
        ),
        (
            DownloadError::Transport(TransportError::Refused),
            Waiting::KeepsChanging,
        ),
        (io(std::io::ErrorKind::NotFound), Waiting::Folder),
        (io(std::io::ErrorKind::PermissionDenied), Waiting::Folder),
        (io(std::io::ErrorKind::StorageFull), Waiting::DiskFull),
    ];
    for (error, expected) in cases {
        match after_error(&error, None, Duration::ZERO) {
            Next::Wait(why, _) => assert_eq!(why, expected, "{error:?}"),
            other => panic!("{error:?}: {other:?}"),
        }
    }
    // Waited for, with what happened in words.
    for error in [
        DownloadError::Transport(TransportError::Corrupt),
        DownloadError::Transport(TransportError::OwnerFailed),
        DownloadError::UnsafePath {
            path: "x".to_owned(),
            reason: "no",
        },
        DownloadError::Transport(TransportError::Protocol("odd".to_owned())),
        DownloadError::Transport(TransportError::Storage("full".to_owned())),
        io(std::io::ErrorKind::TimedOut),
    ] {
        assert!(
            matches!(
                after_error(&error, None, Duration::ZERO),
                Next::Wait(
                    Waiting::Damaged(_)
                        | Waiting::CannotTake(_)
                        | Waiting::Sender(_)
                        | Waiting::ThisComputer(_),
                    _
                )
            ),
            "{error:?}"
        );
    }
}

#[test]
fn a_version_that_does_not_fit_waits_and_says_how_much() {
    let refused = SpaceError::DoesNotFit {
        needed: 12,
        available: 3,
    };
    assert_eq!(
        after_error(&DownloadError::Cancelled, Some(&refused), Duration::ZERO),
        Next::Wait(
            Waiting::Room {
                needed: 12,
                available: 3
            },
            LONGEST_WAIT
        )
    );
}

#[test]
fn what_needs_the_user_stops() {
    for error in [
        DownloadError::Record(RecordError::BadSignature),
        DownloadError::BeingForgotten,
        DownloadError::Cancelled,
        DownloadError::Transport(TransportError::Cancelled),
    ] {
        assert_eq!(
            after_error(&error, None, Duration::ZERO),
            Next::Stop,
            "{error:?}"
        );
    }
}

#[test]
fn a_folder_is_the_sticks_only_with_its_mark() {
    let folder = tempfile::tempdir().unwrap();
    let mine = crate::stick_id::StickSigningKey::from_bytes(&[1; 32]).stick_id();
    let other = crate::stick_id::StickSigningKey::from_bytes(&[2; 32]).stick_id();
    assert!(!received_folder_there(folder.path(), &mine));
    mark_received_folder(folder.path(), &mine).unwrap();
    assert!(received_folder_there(folder.path(), &mine));
    assert!(!received_folder_there(folder.path(), &other));
    assert!(!received_folder_there(&folder.path().join("gone"), &mine));
}

#[tokio::test]
async fn a_cancel_reaches_whoever_waits_before_or_after() {
    let cancel = Cancel::new();
    let waiting = {
        let cancel = cancel.clone();
        tokio::spawn(async move { cancel.cancelled().await })
    };
    tokio::task::yield_now().await;
    cancel.cancel();
    waiting.await.unwrap();
    // Asked after: returns at once.
    cancel.cancelled().await;
    assert!(cancel.is_cancelled());
    assert!(!pause(Duration::from_secs(3600), &cancel).await);
}

#[test]
fn a_ledger_that_fails_is_said_and_tried_again() {
    let failed = SpaceError::Io {
        path: "/data".into(),
        source: std::io::Error::from(std::io::ErrorKind::Other),
    };
    assert!(matches!(
        after_error(&DownloadError::Cancelled, Some(&failed), Duration::ZERO),
        Next::Wait(Waiting::ThisComputer(_), _)
    ));
}

#[test]
fn a_round_writes_into_the_windows_folder_only_while_it_bears_its_mark() {
    let mount_point = tempfile::tempdir().unwrap();
    let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[4; 32]).stick_id();
    // The disk went: its mount point is there, empty.
    assert!(matches!(
        partial_for(mount_point.path(), Some(stick_id)),
        Err(DownloadError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound
    ));
    assert!(std::fs::read_dir(mount_point.path())
        .unwrap()
        .next()
        .is_none());
    mark_received_folder(mount_point.path(), &stick_id).unwrap();
    partial_for(mount_point.path(), Some(stick_id)).unwrap();
    // `get`'s folders bear no mark, and need none.
    let other = tempfile::tempdir().unwrap();
    partial_for(other.path(), None).unwrap();
}

#[test]
fn the_received_folders_mark_is_hidden_and_written_again() {
    let dir = tempfile::tempdir().unwrap();
    let stick_id = crate::stick_id::StickSigningKey::from_bytes(&[7; 32]).stick_id();
    super::mark_received_folder(dir.path(), &stick_id).unwrap();
    // Written again over itself (hidden on Windows, which asks for care).
    super::mark_received_folder(dir.path(), &stick_id).unwrap();
    assert!(super::received_folder_there(dir.path(), &stick_id));
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let mark = std::fs::metadata(dir.path().join(super::RECEIVED_MARKER)).unwrap();
        assert_ne!(mark.file_attributes() & 0x2, 0, "the mark is not hidden");
    }
    // An older, visible mark (written plainly) is hidden, and still reads.
    let older = tempfile::tempdir().unwrap();
    let hex = data_encoding::HEXLOWER.encode(&stick_id.to_bytes());
    std::fs::write(older.path().join(super::RECEIVED_MARKER), &hex).unwrap();
    super::keep_mark_hidden(older.path());
    assert!(super::received_folder_there(older.path(), &stick_id));
    assert!(!older.path().join(".ourstick-received.new").exists());
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let mark = std::fs::metadata(older.path().join(super::RECEIVED_MARKER)).unwrap();
        assert_ne!(
            mark.file_attributes() & 0x2,
            0,
            "the older mark is not hidden"
        );
    }
}
