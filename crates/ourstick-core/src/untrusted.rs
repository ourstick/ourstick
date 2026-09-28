//! Text that comes from a peer (file names in a manifest) is shown to users in
//! events and error messages. It must never reach a terminal or log as control
//! codes, e.g. an escape sequence hidden in a file name.

/// Shows every control character (including ESC and the C1 range) as `\u{..}`,
/// leaving all other text as it is.
pub fn escape(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                format!("\\u{{{:x}}}", character as u32)
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_codes_are_shown_not_sent() {
        assert_eq!(escape("photo.jpg"), "photo.jpg");
        assert_eq!(escape("ünïcödé/ok"), "ünïcödé/ok");
        assert_eq!(escape("\u{1b}[2Jevil"), "\\u{1b}[2Jevil");
        assert_eq!(escape("a\nb\rc\u{7}"), "a\\u{a}b\\u{d}c\\u{7}");
        assert_eq!(escape("\u{9b}31m"), "\\u{9b}31m");
    }
}
