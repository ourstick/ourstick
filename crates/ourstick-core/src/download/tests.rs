use super::*;

#[test]
fn cancellation_maps_to_its_own_error() {
    assert!(matches!(
        cancelled(TransportError::Cancelled),
        DownloadError::Cancelled
    ));
    assert!(matches!(
        cancelled(TransportError::Corrupt),
        DownloadError::Transport(TransportError::Corrupt)
    ));
}
