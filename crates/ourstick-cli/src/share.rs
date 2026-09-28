//! `ourstick share <folder>`: encrypt the folder into the cache, print a
//! ticket, and serve until Ctrl-C.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use ourstick_core::clock::SystemClock;
use ourstick_core::secrets::SecretName;
use ourstick_core::share::{plan_share, publish_folder};
use ourstick_core::transport::iroh::{IrohConfig, IrohPublisher, Network};
use ourstick_core::transport::{describe_network, Publisher};
use ourstick_core::{
    cipher_for, CipherId, ManifestHint, OwnedStick, StickKey, StickSigningKey, Ticket,
};

use crate::display::bytes;
use crate::setup::{self, hex, load_key, now_ms};

const CIPHER: CipherId = CipherId::XChaCha20Poly1305;

/// Finds the stick already made for `folder`, or makes one with new keys.
fn owned_stick_for(
    setup: &setup::Setup,
    folder: &Path,
) -> Result<(OwnedStick, StickSigningKey, StickKey)> {
    if let Some(stick) = setup
        .store
        .owned_sticks()?
        .into_iter()
        .find(|stick| stick.root_path == folder)
    {
        let signing = load_key(
            setup.secrets.as_ref(),
            &SecretName::StickSigningKey(stick.stick_id),
        )?;
        let stick_key = load_key(
            setup.secrets.as_ref(),
            &SecretName::StickKey(stick.stick_id),
        )?;
        return Ok((
            stick,
            StickSigningKey::from_bytes(&signing),
            StickKey::from_bytes(stick_key),
        ));
    }
    let signing_key = StickSigningKey::generate();
    let stick_key = StickKey::generate();
    let stick_id = signing_key.stick_id();
    // Keys first: a stick in the database must always have its keys.
    setup.secrets.put(
        &SecretName::StickSigningKey(stick_id),
        &signing_key.to_bytes(),
    )?;
    setup
        .secrets
        .put(&SecretName::StickKey(stick_id), stick_key.as_bytes())?;
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stick".to_owned());
    let stick = OwnedStick::new(stick_id, name, folder, CIPHER, now_ms());
    setup.store.add_owned_stick(&stick)?;
    Ok((stick, signing_key, stick_key))
}

pub async fn run(folder: &Path) -> Result<()> {
    let folder = std::fs::canonicalize(folder)
        .with_context(|| format!("cannot open {}", folder.display()))?;
    let (setup, stick, signing_key, stick_key) = tokio::task::spawn_blocking({
        let folder = folder.clone();
        move || -> Result<_> {
            let setup = setup::open()?;
            let (stick, signing_key, stick_key) = owned_stick_for(&setup, &folder)?;
            Ok((setup, stick, signing_key, stick_key))
        }
    })
    .await??;

    let cache_dir = setup
        .data_dir
        .join("cache")
        .join(hex(&stick.stick_id.to_bytes()));
    let plan = {
        let folder = folder.clone();
        let cipher = cipher_for(stick.cipher, &stick_key);
        tokio::task::spawn_blocking(move || plan_share(&folder, cipher.as_ref())).await??
    };
    println!(
        "Sharing {} ({} files, {}).",
        folder.display(),
        plan.files,
        bytes(plan.plaintext_bytes)
    );
    println!(
        "The encrypted cache will use about {} in {}",
        bytes(plan.cache_bytes),
        cache_dir.display()
    );

    let config = IrohConfig::new(setup.node_key, Network::Public);
    let publisher = IrohPublisher::start(&config, stick.stick_id, &cache_dir).await?;
    println!("Encrypting…");
    let publication = publish_folder(
        &publisher,
        setup.store.clone(),
        &SystemClock,
        &signing_key,
        &stick_key,
    )
    .await?;
    if !publisher.online(Duration::from_secs(30)).await {
        println!("Warning: no relay reachable; the ticket only has direct addresses.");
    }
    if let Some(report) = publisher.network_report() {
        println!("This network: {}.", describe_network(&report));
    }

    let ticket = Ticket {
        stick_id: stick.stick_id,
        node_id: publisher.node_id(),
        manifest_hint: ManifestHint {
            seq: publication.seq,
            hash: publication.manifest_hash,
        },
        address_hints: publisher.address_hints(),
        stick_key,
    };
    // Printed for the user only; never logged.
    println!(
        "\nTicket (anyone with it can read this stick):\n\n{}\n",
        ticket.to_link()?
    );
    println!("Serving. Press Ctrl-C to stop.");

    tokio::signal::ctrl_c().await?;
    println!("Stopping…");
    publisher.shutdown().await;
    Ok(())
}
