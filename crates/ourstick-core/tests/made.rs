//! ADR 0017, two peers: the owner serves its files' blobs from its own
//! store, made from the files when asked, with no encrypted copies and no
//! suitcases at all; the reader gets every file whole, small and big.

mod common;

use common::{keep_going, noise, Owner, Reader};
use ourstick_core::cipher::cipher_for;
use ourstick_core::share::served_files;
use ourstick_core::transport::Publisher;
use ourstick_core::CipherId;

#[tokio::test(flavor = "multi_thread")]
async fn a_folder_comes_whole_from_blobs_made_when_asked() {
    let big = noise(3 * 1024 * 1024 + 7, 1);
    let files: Vec<(String, Vec<u8>)> = (0..200)
        .map(|index| {
            (
                format!("small/{index:03}.txt"),
                noise(100 + index * 37, index as u8),
            )
        })
        .chain([("big.bin".to_owned(), big)])
        .chain([("empty".to_owned(), Vec::new())])
        .collect();
    let listed: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    // No keeper: nothing would be served but the manifest.
    let owner = Owner::bare(&listed).await;
    let publication = owner.publish().await;
    let stick_id = owner.signing_key.stick_id();
    let table = served_files(
        owner.store.as_ref(),
        &stick_id,
        &publication.manifest.entries,
    )
    .unwrap();
    assert_eq!(table.files.len(), files.len());
    owner.publisher.serve_files(
        cipher_for(CipherId::XChaCha20Poly1305, &owner.stick_key),
        table,
    );

    let reader = Reader::new();
    reader
        .download(&owner.ticket(&publication), &mut keep_going)
        .await
        .unwrap();
    for (name, bytes) in &files {
        assert_eq!(&reader.read(name), bytes, "{name}");
    }
    // No suitcase was ever made.
    let suitcases = owner.publisher.cache_dir().join("suitcases");
    assert_eq!(std::fs::read_dir(suitcases).map_or(0, |dir| dir.count()), 0);
    owner.publisher.shutdown().await;
}
