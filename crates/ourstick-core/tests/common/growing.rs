//! A stick whose owner is still cataloguing it (ADR 0007): versions that list
//! a growing prefix of its files, marked partial, then a complete one.

use std::io::Cursor;
use std::sync::{Arc, Mutex};

use ourstick_core::{cipher_for, encrypt_blob, CipherId, Manifest};

use super::memory::Served;
use super::published::{publish_files, Published};

/// The stick, and what the owner serves at each version.
pub struct Growing {
    pub stick: Published,
    pub versions: Vec<Served>,
}

impl Growing {
    /// The owner now serves version `index`.
    pub fn publish(&self, index: usize) {
        *self.stick.served.lock().unwrap() = self.versions[index].clone();
    }

    pub fn served(&self) -> &Arc<Mutex<Served>> {
        &self.stick.served
    }
}

/// Version `i` lists the first `prefixes[i]` files (in manifest order), at seq
/// 10_000 + i, partial except the last. Every file's blob is in the owner's
/// cache from the start; the first version is served, and is the ticket's
/// hint.
pub async fn grow(files: &[(&str, &[u8])], prefixes: &[usize]) -> Growing {
    let mut stick = publish_files(files).await;
    let full = stick.served.lock().unwrap().clone();
    let cipher = cipher_for(CipherId::XChaCha20Poly1305, &stick.ticket.stick_key);
    let mut versions = Vec::new();
    let mut first_hash = None;
    for (index, &count) in prefixes.iter().enumerate() {
        let manifest = Manifest {
            partial: index + 1 < prefixes.len(),
            name: String::new(),
            entries: stick.manifest.entries[..count].to_vec(),
        };
        let mut blob = Vec::new();
        encrypt_blob(
            cipher.as_ref(),
            &mut Cursor::new(manifest.to_bytes().unwrap()),
            &mut blob,
        )
        .unwrap();
        let hash = *blake3::hash(&blob).as_bytes();
        first_hash.get_or_insert(hash);
        let mut served = full.clone();
        served.blobs.insert(hash, blob);
        served.allowed = manifest.entries.iter().map(|entry| entry.hash).collect();
        served.allowed.insert(hash);
        served.record = Some(stick.signing_key.sign_record(10_000 + index as u64, hash));
        versions.push(served);
    }
    stick.ticket.manifest_hint = ourstick_core::ManifestHint {
        seq: 10_000,
        hash: first_hash.unwrap(),
    };
    let growing = Growing { stick, versions };
    growing.publish(0);
    growing
}
