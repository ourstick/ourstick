//! The line for a reader kept on the relay because macOS declines Terminal
//! the local network (ADR 0020, rule 7).

use std::time::Duration;

use ourstick_core::transport::ReaderActivity;

use crate::progress::clock;

/// Said once it starts: a reader on this network, kept on the relay because
/// macOS does not let Terminal (whose permission the CLI has) reach it.
pub(super) fn local_network_line(
    before: bool,
    now: &ReaderActivity,
    elapsed: Duration,
) -> Option<String> {
    (now.local_network_declined && !before).then(|| {
        format!(
            "[{}] A reader is on this network, but macOS does not let Terminal reach it: \
             allow it in System Settings → Privacy & Security → Local Network.",
            clock(elapsed)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_said_when_it_starts_only() {
        let declined = ReaderActivity {
            local_network_declined: true,
            ..ReaderActivity::default()
        };
        let at = Duration::from_secs(70);
        let line = local_network_line(false, &declined, at).unwrap();
        assert!(line.starts_with("[01:10] ") && line.contains("Terminal"));
        assert_eq!(local_network_line(true, &declined, at), None);
        assert_eq!(
            local_network_line(false, &ReaderActivity::default(), at),
            None
        );
    }
}
