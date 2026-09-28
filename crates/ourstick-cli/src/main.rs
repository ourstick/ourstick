//! `ourstick` CLI: share / get.

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
  ourstick get <ticket> [--out <folder>]   Download a stick (resumes if interrupted)

Quote the ticket, e.g. ourstick get 'https://ourstick.com/s/…#…'";

enum Command {
    Share {
        folder: PathBuf,
    },
    Get {
        ticket: String,
        output: Option<PathBuf>,
    },
    Help,
}

fn parse(args: &[String]) -> Result<Command> {
    match args {
        [command, folder] if command == "share" => Ok(Command::Share {
            folder: folder.into(),
        }),
        [command, ticket] if command == "get" => Ok(Command::Get {
            ticket: ticket.clone(),
            output: None,
        }),
        [command, ticket, flag, output] if command == "get" && flag == "--out" => {
            Ok(Command::Get {
                ticket: ticket.clone(),
                output: Some(output.into()),
            })
        }
        [] => Ok(Command::Help),
        [flag] if flag == "-h" || flag == "--help" || flag == "help" => Ok(Command::Help),
        _ => bail!("unknown command\n\n{USAGE}"),
    }
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
        Command::Get { ticket, output } => get::run(&ticket, output).await,
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
            Ok(Command::Get { ticket, output: None }) if ticket == "T"
        ));
        assert!(matches!(
            parse(&args(&["get", "T", "--out", "dir"])),
            Ok(Command::Get { output: Some(output), .. }) if output == std::path::Path::new("dir")
        ));
        assert!(matches!(parse(&args(&[])), Ok(Command::Help)));
        assert!(matches!(parse(&args(&["--help"])), Ok(Command::Help)));
        assert!(parse(&args(&["share"])).is_err());
        assert!(parse(&args(&["get", "T", "--in", "dir"])).is_err());
    }
}
