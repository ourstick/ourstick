//! `ourstick ls <ticket>`: show a stick's files without downloading them.

use anyhow::Result;
use ourstick_core::download::{list, Listing};
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
use ourstick_core::Ticket;

use crate::display::{bytes, untrusted};
use crate::setup::{self, hex};
use ourstick_core::device::random_key;

/// One line per file (size, then path), then the total. Paths come from the
/// owner, so they are escaped.
pub fn listing_lines(listing: &Listing) -> Vec<String> {
    let entries = &listing.manifest.entries;
    let sizes: Vec<String> = entries.iter().map(|entry| bytes(entry.size)).collect();
    let width = sizes.iter().map(|size| size.len()).max().unwrap_or(0);
    let mut lines: Vec<String> = entries
        .iter()
        .zip(&sizes)
        .map(|(entry, size)| format!("{size:>width$}  {}", untrusted(&entry.path)))
        .collect();
    let files = match entries.len() {
        1 => "1 file".to_owned(),
        count => format!("{count} files"),
    };
    lines.push(format!(
        "{files}, {}.",
        bytes(listing.manifest.total_size())
    ));
    lines
}

pub async fn run(ticket_text: &str) -> Result<()> {
    let ticket: Ticket = ticket_text.parse()?;
    let setup = tokio::task::spawn_blocking(setup::open_for_reading).await??;
    // Its own blob store, apart from a `get` of the same stick that may be
    // running; only the manifest lands in it, and it is removed afterwards.
    let store_dir = setup
        .data_dir
        .join("listing")
        .join(hex(&ticket.stick_id.to_bytes()));
    let config = IrohConfig::new(random_key(), Network::Public);
    let fetcher = IrohFetcher::start(&config, &store_dir).await?;
    println!("Connecting to the owner…");

    let result = tokio::select! {
        result = list(&fetcher, setup.store.clone(), &ticket) => Some(result),
        _ = tokio::signal::ctrl_c() => None,
    };
    fetcher.shutdown(false).await?;
    match result {
        Some(listing) => {
            for line in listing_lines(&listing?) {
                println!("{line}");
            }
            Ok(())
        }
        None => std::process::exit(130),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::manifest::{Entry, Manifest};
    use ourstick_core::ManifestVersion;

    fn listing(entries: &[(&str, u64)]) -> Listing {
        Listing {
            version: ManifestVersion {
                seq: 1,
                hash: [0; 32],
            },
            manifest: Manifest {
                entries: entries
                    .iter()
                    .map(|(path, size)| Entry {
                        path: (*path).to_owned(),
                        size: *size,
                        mtime: 0,
                        hash: [0; 32],
                    })
                    .collect(),
                ..Manifest::default()
            },
        }
    }

    #[test]
    fn sizes_are_aligned_and_the_total_follows() {
        assert_eq!(
            listing_lines(&listing(&[("a/big.bin", 300_000_000), ("note.txt", 512)])),
            vec![
                "300.0 MB  a/big.bin",
                "   512 B  note.txt",
                "2 files, 300.0 MB."
            ]
        );
        assert_eq!(
            listing_lines(&listing(&[("x", 1)])),
            vec!["1 B  x", "1 file, 1 B."]
        );
        assert_eq!(listing_lines(&listing(&[])), vec!["0 files, 0 B."]);
    }

    #[test]
    fn names_from_the_owner_are_escaped() {
        let lines = listing_lines(&listing(&[("\u{1b}[2Jevil\n.txt", 1)]));
        assert_eq!(lines[0], "1 B  \\u{1b}[2Jevil\\u{a}.txt");
    }
}
