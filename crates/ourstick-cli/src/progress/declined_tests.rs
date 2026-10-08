use super::*;

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

#[test]
fn a_declined_local_network_names_terminal_and_where_to_allow_it() {
    let mut progress = Progress::default();
    let lines = progress.on_event(
        DownloadEvent::LocalNetworkDeclined { declined: true },
        secs(12),
    );
    let [line] = lines.as_slice() else {
        panic!("one line")
    };
    assert!(line.starts_with("[00:12] ") && line.contains("Terminal") && line.contains("Local"));
    let allowed = DownloadEvent::LocalNetworkDeclined { declined: false };
    assert!(progress.on_event(allowed, secs(30)).is_empty());
    let again = DownloadEvent::LocalNetworkDeclined { declined: true };
    assert!(progress.on_event(again, secs(40)).is_empty(), "once a run");
}
