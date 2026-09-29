//! `ourstick` CLI: share / ls / get.

mod display;
mod get;
mod ls;
mod progress;
mod qr;
mod secrets;
mod setup;
mod share;

use std::path::PathBuf;

use anyhow::{bail, Result};

use secrets::SecretsChoice;

const USAGE: &str = "\
ourstick — a USB stick you can send

Usage:
  ourstick share [--key-file <path>] [--rescan] <folder>
                                           Share a folder and print its ticket
  ourstick ls <ticket>                     List a stick's files without downloading
  ourstick get <ticket> [--out <folder>] [--follow]
                                           Download a stick (resumes if interrupted;
                                           later runs fetch only what changed)

Without a direct connection, `get` goes on through a relay, at up to 10 Mbps
while it keeps looking for a direct one; it never stops because of it.

`share` keeps this device's keys in the Secret Service (GNOME Keyring,
KWallet). Where there is none (macOS for now, SSH sessions, servers), the first
`share --key-file <path>` keeps them in a file protected by a passphrase
instead; later runs use that file and ask for its passphrase.

`share` encrypts only files whose size and modification times changed since
the last share; --rescan reads and encrypts every file.

`get` never overwrites a file changed in its folder, and keeps files that left
the stick. With --follow it keeps running and downloads every new version.

Quote the ticket, e.g. ourstick get 'https://ourstick.com/s/…#…'";

enum Command {
    Share {
        folder: PathBuf,
        secrets: SecretsChoice,
        rescan: bool,
    },
    List {
        ticket: String,
    },
    Get {
        ticket: String,
        output: Option<PathBuf>,
        follow: bool,
    },
    Help,
}

fn parse(args: &[String]) -> Result<Command> {
    match args {
        [command, options @ ..] if command == "share" => parse_share(options),
        [command, ticket] if command == "ls" => Ok(Command::List {
            ticket: ticket.to_owned(),
        }),
        [command, ticket, options @ ..] if command == "get" => parse_get(ticket, options),
        [] => Ok(Command::Help),
        [flag] if flag == "-h" || flag == "--help" || flag == "help" => Ok(Command::Help),
        _ => bail!("unknown command\n\n{USAGE}"),
    }
}

/// `share <folder>`, with `--key-file <path>` and/or `--rescan` before or
/// after it.
fn parse_share(options: &[String]) -> Result<Command> {
    let mut folder = None;
    let mut key_file = None;
    let mut rescan = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--key-file" => match options.next() {
                Some(path) if key_file.is_none() => key_file = Some(PathBuf::from(path)),
                _ => bail!("--key-file needs one path\n\n{USAGE}"),
            },
            "--rescan" => rescan = true,
            other if other.starts_with("--") => bail!("unknown option {other}\n\n{USAGE}"),
            _ if folder.is_some() => bail!("share takes one folder\n\n{USAGE}"),
            path => folder = Some(PathBuf::from(path)),
        }
    }
    let Some(folder) = folder else {
        bail!("share needs a folder\n\n{USAGE}");
    };
    Ok(Command::Share {
        folder,
        secrets: key_file.map_or(SecretsChoice::Default, SecretsChoice::KeyFile),
        rescan,
    })
}

/// `get <ticket>` followed by `--out <folder>` and/or `--allow-relay`, in any order.
fn parse_get(ticket: &str, options: &[String]) -> Result<Command> {
    let mut output = None;
    let mut follow = false;
    let mut options = options.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--out" => match options.next() {
                Some(folder) if output.is_none() => output = Some(PathBuf::from(folder)),
                _ => bail!("--out needs one folder\n\n{USAGE}"),
            },
            "--follow" => follow = true,
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
    }
    Ok(Command::Get {
        ticket: ticket.to_owned(),
        output,
        follow,
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
        Command::Share {
            folder,
            secrets,
            rescan,
        } => share::run(&folder, secrets, rescan).await,
        Command::List { ticket } => ls::run(&ticket).await,
        Command::Get {
            ticket,
            output,
            follow,
        } => get::run(&ticket, output, follow).await,
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
            Ok(Command::Share { folder, secrets: SecretsChoice::Default, rescan: false })
                if folder == std::path::Path::new("photos")
        ));
        assert!(matches!(
            parse(&args(&["share", "--rescan", "photos"])),
            Ok(Command::Share { rescan: true, .. })
        ));
        for order in [
            &["share", "--key-file", "k.oskf", "photos"],
            &["share", "photos", "--key-file", "k.oskf"],
        ] {
            assert!(matches!(
                parse(&args(order)),
                Ok(Command::Share { folder, secrets: SecretsChoice::KeyFile(path), .. })
                    if folder == std::path::Path::new("photos")
                        && path == std::path::Path::new("k.oskf")
            ));
        }
        assert!(parse(&args(&["share", "a", "b"])).is_err());
        assert!(parse(&args(&["share", "--key-file"])).is_err());
        assert!(parse(&args(&["share", "--key-file", "k"])).is_err());
        assert!(parse(&args(&["share", "--key-file", "k", "--key-file", "j", "a"])).is_err());
        assert!(parse(&args(&["share", "--keyfile", "k", "a"])).is_err());
        assert!(matches!(
            parse(&args(&["get", "T"])),
            Ok(Command::Get { ticket, output: None, follow: false })
                if ticket == "T"
        ));
        assert!(matches!(
            parse(&args(&["get", "T", "--out", "dir"])),
            Ok(Command::Get { output: Some(output), .. }) if output == std::path::Path::new("dir")
        ));
        assert!(matches!(
            parse(&args(&["ls", "T"])),
            Ok(Command::List { ticket }) if ticket == "T"
        ));
        assert!(parse(&args(&["ls"])).is_err());
        assert!(parse(&args(&["ls", "T", "--out", "dir"])).is_err());
        assert!(matches!(parse(&args(&[])), Ok(Command::Help)));
        assert!(matches!(parse(&args(&["--help"])), Ok(Command::Help)));
        assert!(parse(&args(&["share"])).is_err());
        assert!(parse(&args(&["get", "T", "--in", "dir"])).is_err());
        assert!(matches!(
            parse(&args(&["get", "T", "--follow", "--out", "dir"])),
            Ok(Command::Get { output: Some(output), follow: true, .. })
                if output == std::path::Path::new("dir")
        ));
        // Gone with ADR 0006 v2: the relay is never refused.
        assert!(parse(&args(&["get", "T", "--allow-relay"])).is_err());
        assert!(matches!(
            parse(&args(&["get", "T", "--follow"])),
            Ok(Command::Get { follow: true, .. })
        ));
        assert!(parse(&args(&["get", "T", "--out"])).is_err());
        assert!(parse(&args(&["get", "T", "--out", "a", "--out", "b"])).is_err());
    }
}
