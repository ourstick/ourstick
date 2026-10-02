//! Text that comes from a peer (file names in a manifest) is shown to users in
//! events and error messages. It must never reach a terminal or log as control
//! codes, e.g. an escape sequence hidden in a file name.

/// Shows every control character (including ESC and the C1 range) and
/// every character that overrides or isolates the writing direction
/// (U+202A–202E, U+2066–2069) as `\u{..}`, leaving all other text as it is:
/// emoji joiners, Persian ZWNJ, and the direction marks Hebrew and Arabic
/// names carry (U+200E, U+200F, U+061C), which cannot reorder text.
pub fn escape(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || turns_direction(character) {
                format!("\\u{{{:x}}}", character as u32)
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// Whether `character` changes the direction text is shown in.
fn turns_direction(character: char) -> bool {
    matches!(
        character,
        '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
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

    #[test]
    fn a_name_cannot_turn_the_text_around() {
        assert_eq!(escape("photo\u{202e}gpj.exe"), "photo\\u{202e}gpj.exe");
        assert_eq!(escape("a\u{2066}b"), "a\\u{2066}b");
        assert_eq!(escape("\u{202a}x\u{2069}"), "\\u{202a}x\\u{2069}");
        // Direction marks stay: Hebrew and Arabic names carry them.
        assert_eq!(escape("\u{5e9}\u{200f}.txt"), "\u{5e9}\u{200f}.txt");
        // Joiners stay: they are part of words and emoji.
        assert_eq!(
            escape("\u{1f469}\u{200d}\u{1f4bb}"),
            "\u{1f469}\u{200d}\u{1f4bb}"
        );
        assert_eq!(escape("\u{645}\u{200c}\u{646}"), "\u{645}\u{200c}\u{646}");
    }
}
