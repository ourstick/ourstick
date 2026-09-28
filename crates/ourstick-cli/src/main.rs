//! `ourstick` CLI: share / get.

mod consent;
mod display;
mod get;
mod progress;
mod setup;
mod share;

use std::path::PathBuf;

use anyhow::{bail, Result};

const USAGE: &str = "\
ourstick — a USB stick you can send

Usage:
  ourstick share <folder>                  Share a folder and print its ticket
  ourstick get <ticket> [--out <folder>] [--allow-relay]
                                           Download a stick (resumes if interrupted)

`get` waits up to 5 s for a direct connection, then starts. Up to 100 MB may
come through a relay while it keeps looking; after that it waits up to 15 s
more and asks before going on through the relay. --allow-relay says yes in
advance (for scripts).

Quote the ticket, e.g. ourstick get 'https://ourstick.com/s/…#…'";

enum Command {
    Share {
        folder: PathBuf,
    },
    Get {
        ticket: String,
        output: Option<PathBuf>,
        allow_relay: bool,
    },
    Help,
}

fn parse(args: &[String]) -> Result<Command> {
    match args {
        [command, folder] if command == "share" => Ok(Command::Share {
            folder: folder.into(),
        }),
        [command, ticket, options @ ..] if command == "get" => parse_get(ticket, options),
        [] => Ok(Command::Help),
        [flag] if flag == "-h" || flag == "--help" || flag == "help" => Ok(Command::Help),
        _ => bail!("unknown command\n\n{USAGE}"),
    }
}

/// `get <ticket>` followed by `--out <folder>` and/or `--allow-relay`, in any order.
fn parse_get(ticket: &str, options: &[String]) -> Result<Command> {
    let mut output = None;
    let mut allow_relay = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--out" => match options.next() {
                Some(folder) if output.is_none() => output = Some(PathBuf::from(folder)),
                _ => bail!("--out needs one folder\n\n{USAGE}"),
            },
            "--allow-relay" => allow_relay = true,
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
    }
    Ok(Command::Get {
        ticket: ticket.to_owned(),
        output,
        allow_relay,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args)? {
        Command::Share { folder } => share::run(&folder).await,
        Command::Get {
            ticket,
            output,
            allow_relay,
        } => get::run(&ticket, output, allow_relay).await,
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn parses_commands() {
        assert!(matches!(
            parse(&args(&["share", "photos"])),
            Ok(Command::Share { folder }) if folder == std::path::Path::new("photos")
        ));
        assert!(matches!(
            parse(&args(&["get", "T"])),
            Ok(Command::Get { ticket, output: None, allow_relay: false }) if ticket == "T"
        ));
        assert!(matches!(
            parse(&args(&["get", "T", "--out", "dir"])),
            Ok(Command::Get { output: Some(output), .. }) if output == std::path::Path::new("dir")
        ));
        assert!(matches!(parse(&args(&[])), Ok(Command::Help)));
        assert!(matches!(parse(&args(&["--help"])), Ok(Command::Help)));
        assert!(parse(&args(&["share"])).is_err());
        assert!(parse(&args(&["get", "T", "--in", "dir"])).is_err());
        assert!(matches!(
            parse(&args(&["get", "T", "--allow-relay", "--out", "dir"])),
            Ok(Command::Get { output: Some(output), allow_relay: true, .. })
                if output == std::path::Path::new("dir")
        ));
        assert!(parse(&args(&["get", "T", "--out"])).is_err());
        assert!(parse(&args(&["get", "T", "--out", "a", "--out", "b"])).is_err());
    }
}
