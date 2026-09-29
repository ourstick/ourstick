//! OurStick core: a USB stick you can send.
//!
//! This crate holds all domain logic and depends on no UI.
//! See CLAUDE.md at the repository root for the architecture rules.

pub mod blob;
mod blocking;
pub mod cipher;
pub mod clock;
pub mod download;
pub mod manifest;
pub mod record;
pub mod secrets;
pub mod share;
pub mod stick_id;
pub mod store;
pub mod ticket;
pub mod transport;
pub mod untrusted;
pub mod watcher;

pub use blob::{
    blob_len, decrypt_blob, encrypt_blob, BlobHeader, BlobInfo, BLOB_FORMAT_VERSION, CHUNK_SIZE,
};
pub use cipher::{cipher_for, ChaChaCipher, Cipher, CipherError, CipherId, NoCipher, StickKey};
pub use manifest::{Entry, Manifest, ManifestError, MANIFEST_FORMAT_VERSION};
pub use record::{
    next_seq, ManifestHint, ManifestRecord, RecordCheck, RecordError, SignedManifestRecord,
};
pub use secrets::{check_secret_store, SecretError, SecretName, SecretStore};
pub use stick_id::{InvalidPublicKey, StickId, StickSigningKey};
pub use store::{
    Device, JoinedStick, ManifestVersion, MemoryStore, OwnedFile, OwnedStick, SecretBackend,
    SqliteStore, Store, StoreError,
};
pub use ticket::{AddressHint, NodeId, Ticket, TicketError};
