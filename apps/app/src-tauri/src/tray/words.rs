//! The one sentence of each thing in the tray, with its symbol: the same
//! words as the home's rows (`Home.svelte`, `format.ts`).

use crate::receiving::Shown;
use crate::state::Sharing;

/// Something sent: how it goes now.
pub fn sending(sharing: Option<&Sharing>) -> String {
    match sharing {
        Some(Sharing::On(serving)) => match serving.activity.borrow().readers {
            0 => "Nobody is getting it right now".to_owned(),
            1 => "\u{2191} Someone is getting it".to_owned(),
            readers => format!("\u{2191} {readers} people are getting it"),
        },
        Some(Sharing::Starting) => "Getting it ready\u{2026}".to_owned(),
        Some(Sharing::Busy(doing)) => (*doing).to_owned(),
        Some(Sharing::Failed(_)) => "\u{2715} Stopped: needs you".to_owned(),
        Some(Sharing::Off) | None => "\u{23f8} Stopped by you".to_owned(),
    }
}

/// Something received, or a link waited for: how it goes now.
pub fn received(shown: &Shown, waiting: bool) -> String {
    if waiting {
        return "\u{2026} Waiting for the sender".to_owned();
    }
    match shown.state {
        "downloading" if shown.total > 0 => format!(
            "\u{2193} Downloading {}%",
            shown.done.saturating_mul(100) / shown.total
        ),
        "downloading" => "\u{2193} Downloading\u{2026}".to_owned(),
        "getting_more" => "\u{2713} All here so far".to_owned(),
        "up_to_date" => "\u{2713} All here".to_owned(),
        "waiting" => "! Waiting".to_owned(),
        "paused" => "\u{23f8} Paused".to_owned(),
        "stopped" => "\u{2715} Stopped: needs you".to_owned(),
        _ => "Getting ready\u{2026}".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_says_how_far_it_is() {
        let shown = Shown {
            state: "downloading",
            done: 45,
            total: 100,
            ..Shown::default()
        };
        assert_eq!(received(&shown, false), "\u{2193} Downloading 45%");
        assert_eq!(received(&shown, true), "\u{2026} Waiting for the sender");
        let all = Shown {
            state: "up_to_date",
            ..Shown::default()
        };
        assert_eq!(received(&all, false), "\u{2713} All here");
    }

    #[test]
    fn sending_says_who_gets_it_or_why_not() {
        assert_eq!(sending(None), "\u{23f8} Stopped by you");
        assert_eq!(
            sending(Some(&Sharing::Starting)),
            "Getting it ready\u{2026}"
        );
        assert_eq!(
            sending(Some(&Sharing::Busy("Renaming\u{2026}"))),
            "Renaming\u{2026}"
        );
    }
}
