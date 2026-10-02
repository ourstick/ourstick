use super::*;
use crate::record::RecordError;

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
        DownloadError::PathCollision {
            first: "a".to_owned(),
            second: "A".to_owned(),
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
