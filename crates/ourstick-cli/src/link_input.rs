//! The link given as `-`: read from standard input, so it is never in the
//! process's arguments, which any user of the computer can read (`ps`,
//! `/proc/<pid>/cmdline`) and whose fragment is the stick's key. From a
//! terminal it is asked for without echo, like a passphrase.

use std::io::{IsTerminal, Read};

use anyhow::{bail, Context, Result};

/// `argument` itself, or, for `-`, the link from standard input.
pub fn link_from(argument: &str) -> Result<String> {
    if argument != "-" {
        return Ok(argument.to_owned());
    }
    let read = if std::io::stdin().is_terminal() {
        rpassword::prompt_password("Sharing link (not shown): ")
            .context("the sharing link could not be read from the terminal")?
    } else {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("the sharing link could not be read from standard input")?;
        text
    };
    first_line(&read)
}

/// The link in what was read: its first line that is not blank.
fn first_line(read: &str) -> Result<String> {
    match read.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some(link) => Ok(link.to_owned()),
        None => bail!("no sharing link was given on standard input"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_given_as_an_argument_is_taken_as_it_is() {
        assert_eq!(
            link_from("https://ourstick.com/s/#a.b").unwrap(),
            "https://ourstick.com/s/#a.b"
        );
    }

    #[test]
    fn the_first_line_with_text_is_the_link() {
        assert_eq!(
            first_line("\n  https://ourstick.com/s/#a.b  \nmore\n").unwrap(),
            "https://ourstick.com/s/#a.b"
        );
        assert!(first_line(" \n\n").is_err());
    }
}
