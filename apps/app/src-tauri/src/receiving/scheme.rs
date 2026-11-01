//! The one address OurStick acts on when a browser opens it (ADR 0030):
//! `ourstick:receive`. It carries nothing: no key, no stick, no sender, and
//! says only "show Receive", which then reads the clipboard. Any other
//! `ourstick:` address is refused whole and never read as a link, logged,
//! shown or put into an error: someone may have put a key in it.

use std::ffi::OsStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The forms of the address, compared ignoring ASCII case.
const FORMS: [&str; 2] = ["ourstick:receive", "ourstick://receive"];

/// Arrivals closer than this to the last one are dropped, so that a page
/// cannot keep raising the window.
const GAP: Duration = Duration::from_secs(3);

/// Whether `argument` is `ourstick:receive`, in one of its forms, with or
/// without a trailing `/`.
pub fn is_receive(argument: &OsStr) -> bool {
    let text = argument.to_string_lossy();
    let text = text.strip_suffix('/').unwrap_or(&text);
    FORMS.iter().any(|form| text.eq_ignore_ascii_case(form))
}

/// Whether the arguments a start was given hold `ourstick:receive`. The
/// others are not looked at: an address cannot pass a flag.
pub fn asked<I, S>(arguments: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    arguments
        .into_iter()
        .any(|argument| is_receive(argument.as_ref()))
}

/// When `ourstick:receive` last arrived and was acted on.
#[derive(Default)]
pub struct Arrivals(Mutex<Option<Instant>>);

impl Arrivals {
    /// Whether an arrival at `now` is acted on: not closer than [`GAP`] to
    /// the last one that was.
    pub fn take(&self, now: Instant) -> bool {
        let mut last = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|last| now.saturating_duration_since(last) < GAP) {
            return false;
        }
        *last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn receive(text: &str) -> bool {
        is_receive(OsStr::new(text))
    }

    #[test]
    fn the_four_forms_in_any_case_are_receive() {
        for text in [
            "ourstick:receive",
            "ourstick:receive/",
            "ourstick://receive",
            "ourstick://receive/",
            "OURSTICK:RECEIVE",
            "OurStick://Receive/",
        ] {
            assert!(receive(text), "{text}");
        }
    }

    #[test]
    fn anything_else_is_refused_whole() {
        for text in [
            "",
            "ourstick:",
            "ourstick://",
            "ourstick:receiv",
            "ourstick:receive//",
            "ourstick:receive?x=1",
            "ourstick:receive#abcdefghijklmnopqrstuvwxyz234567.abcdefghij",
            "ourstick://receive/#abc.def",
            "ourstick://receive/more",
            " ourstick:receive",
            "ourstick:receive ",
            "https://ourstick.com/s/#abc.def",
            "receive",
            "--autostarted",
            "file:///home/someone/ourstick:receive",
        ] {
            assert!(!receive(text), "{text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn text_that_is_not_utf8_is_refused_without_a_panic() {
        use std::os::unix::ffi::OsStrExt;
        assert!(!is_receive(OsStr::from_bytes(b"ourstick:rec\xffeive")));
        assert!(!is_receive(OsStr::from_bytes(b"\xff\xfe")));
    }

    #[test]
    fn a_start_asks_only_when_one_argument_is_the_address() {
        let start =
            |arguments: &[&str]| asked(arguments.iter().map(OsString::from).collect::<Vec<_>>());
        assert!(start(&["/usr/bin/ourstick-app", "ourstick:receive"]));
        assert!(start(&[
            "OurStick.exe",
            "--autostarted",
            "ourstick://receive/"
        ]));
        assert!(!start(&["/usr/bin/ourstick-app"]));
        assert!(!start(&["/usr/bin/ourstick-app", "--autostarted"]));
        assert!(!start(&["/usr/bin/ourstick-app", "ourstick:--autostarted"]));
    }

    #[test]
    fn arrivals_closer_than_the_gap_are_dropped() {
        let arrivals = Arrivals::default();
        let first = Instant::now();
        assert!(arrivals.take(first));
        assert!(!arrivals.take(first + Duration::from_secs(1)));
        assert!(!arrivals.take(first + GAP - Duration::from_millis(1)));
        assert!(arrivals.take(first + GAP));
        // Measured from the last one acted on, not the last one dropped.
        assert!(!arrivals.take(first + GAP + Duration::from_secs(2)));
        assert!(arrivals.take(first + GAP + GAP));
    }
}
