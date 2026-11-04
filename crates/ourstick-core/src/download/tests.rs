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

#[test]
fn an_io_error_shows_the_owners_names_escaped() {
    let error = DownloadError::Io {
        path: std::path::PathBuf::from("out/a\nError: forged line\u{1b}[2J"),
        source: std::io::Error::other("full"),
    };
    let shown = error.to_string();
    assert!(
        !shown.contains('\n') && !shown.contains('\u{1b}'),
        "{shown}"
    );
    assert!(shown.contains("a\\u{a}Error: forged line"), "{shown}");
}
