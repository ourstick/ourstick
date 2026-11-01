//! The receiver's words, said for the device they are shown on: a phone
//! (ADR 0027) is "this phone", and it does not go on by itself (it gets a
//! stick only while asked), so it asks for Try again instead.

/// `words` for this device.
pub fn here(words: String) -> String {
    if super::FOLLOWS {
        words
    } else {
        on_a_phone(&words)
    }
}

/// What the window shows of a stick, its message for this device.
pub fn shown_here(mut shown: super::Shown) -> super::Shown {
    shown.message = shown.message.map(here);
    shown
}

fn on_a_phone(words: &str) -> String {
    let mut words = words
        .replace("this computer", "this phone")
        .replace("This computer", "This phone")
        .replace("OurStick tries again by itself", "Use Try again");
    if let Some(at) = words.find("It goes on") {
        let end = words[at..]
            .find(". ")
            .map_or(words.len(), |end| at + end + 2);
        words.replace_range(at..end, "");
        words = words.trim_end().to_owned();
        if !words.contains("Try again") {
            words.push_str(" Use Try again.");
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_phone_is_this_phone_and_asks_for_try_again() {
        assert_eq!(
            on_a_phone(
                "The sender is offline right now. It goes on by itself, all of it, once they \
                 are back."
            ),
            "The sender is offline right now. Use Try again."
        );
        assert_eq!(
            on_a_phone("Not enough room on this computer. It goes on when there is room. Sorry."),
            "Not enough room on this phone. Sorry. Use Try again."
        );
        assert_eq!(
            on_a_phone("Saving it on this computer failed. OurStick tries again by itself."),
            "Saving it on this phone failed. Use Try again."
        );
        assert_eq!(
            on_a_phone("This computer's network could not be started."),
            "This phone's network could not be started."
        );
    }
}
