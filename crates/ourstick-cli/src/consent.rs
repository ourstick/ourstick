//! Asking the user whether a download may continue through the relay once
//! the relay budget is used up (ADR 0006). Default no; `--allow-relay` says yes in advance; without a
//! terminal to ask, the answer is no.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use ourstick_core::download::{RelayConsent, RelayQuestion};

use crate::display::bytes;
use crate::progress::clock;

pub struct CliConsent {
    allow_relay: bool,
    started: Instant,
    /// Set once the reasons for the relay have been printed.
    explained: AtomicBool,
}

impl CliConsent {
    pub fn new(allow_relay: bool, started: Instant) -> Self {
        Self {
            allow_relay,
            started,
            explained: AtomicBool::new(false),
        }
    }

    /// Whether the reasons for using the relay were already shown.
    pub fn explained(&self) -> bool {
        self.explained.load(Ordering::SeqCst)
    }
}

/// The explanation printed before the question.
pub fn question_lines(question: &RelayQuestion, at: &str) -> Vec<String> {
    let what = if question.direct_was_lost {
        "The direct connection was lost and could not be found again"
    } else {
        "No direct connection to the owner could be made"
    };
    let mut lines = vec![format!(
        "[{at}] {what}. {} came through the relay; {} are left to download.",
        bytes(question.relay_used),
        bytes(question.remaining)
    )];
    lines.push("        Likely reasons:".to_owned());
    for reason in &question.reasons {
        lines.push(format!("        - {reason}"));
    }
    lines
}

/// `y` or `yes`, in any letter case; anything else (including nothing) is no.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
}

impl RelayConsent for CliConsent {
    async fn allow_relay(&self, question: RelayQuestion) -> bool {
        self.explained.store(true, Ordering::SeqCst);
        let at = clock(self.started.elapsed());
        for line in question_lines(&question, &at) {
            println!("{line}");
        }
        if self.allow_relay {
            println!(
                "[{at}] Continuing through {} (--allow-relay).",
                question.relay
            );
            return true;
        }
        if !std::io::stdin().is_terminal() {
            println!(
                "[{at}] Not using the relay: there is no terminal to ask. \
                 Run with --allow-relay to allow it."
            );
            return false;
        }
        print!(
            "Continue through {}? It will be slower. [y/N] ",
            question.relay
        );
        let _ = std::io::stdout().flush();
        let answer = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            line
        })
        .await
        .unwrap_or_default();
        is_yes(&answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn question(direct_was_lost: bool) -> RelayQuestion {
        RelayQuestion {
            remaining: 4_500_000_000,
            relay_used: 104_000_000,
            relay: "the public iroh relays run by n0".to_owned(),
            reasons: vec!["reason one".to_owned(), "reason two".to_owned()],
            direct_was_lost,
        }
    }

    #[test]
    fn question_explains_what_happened_and_why() {
        assert_eq!(
            question_lines(&question(false), "00:15"),
            vec![
                "[00:15] No direct connection to the owner could be made. 104.0 MB came through the relay; 4.5 GB are left to download.",
                "        Likely reasons:",
                "        - reason one",
                "        - reason two",
            ]
        );
        assert!(question_lines(&question(true), "01:02")[0]
            .starts_with("[01:02] The direct connection was lost"));
    }

    #[test]
    fn only_yes_means_yes() {
        for answer in ["y", "Y", "yes", " YES \n"] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["", "\n", "n", "no", "ναι", "yess", "maybe"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }
}
