//! Formatting for the terminal.

/// Makes text from a peer (file names in a manifest) safe to print; see
/// [`ourstick_core::untrusted::escape`].
pub fn untrusted(text: &str) -> String {
    ourstick_core::untrusted::escape(text)
}

/// A byte count for humans, e.g. `1.5 MB`.
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = count as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{count} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_text_cannot_reach_the_terminal_as_control_codes() {
        assert_eq!(untrusted("photo.jpg"), "photo.jpg");
        assert_eq!(untrusted("ünïcödé/ok"), "ünïcödé/ok");
        assert_eq!(untrusted("\u{1b}[2Jevil"), "\\u{1b}[2Jevil");
        assert_eq!(untrusted("a\nb\rc\u{7}"), "a\\u{a}b\\u{d}c\\u{7}");
        assert_eq!(untrusted("\u{9b}31m"), "\\u{9b}31m");
    }

    #[test]
    fn bytes_are_readable() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_500), "1.5 KB");
        assert_eq!(bytes(2_000_000), "2.0 MB");
        assert_eq!(bytes(3_210_000_000), "3.2 GB");
    }
}
