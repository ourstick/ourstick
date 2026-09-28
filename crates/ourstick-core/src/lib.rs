//! OurStick core: a USB stick you can send.
//!
//! This crate holds all domain logic and depends on no UI.
//! See CLAUDE.md at the repository root for the architecture rules.

pub mod blob;
pub mod cipher;
pub mod manifest;
pub mod record;
pub mod secrets;
pub mod stick_id;
pub mod ticket;

pub use blob::{decrypt_blob, encrypt_blob, BlobHeader, BlobInfo, BLOB_FORMAT_VERSION, CHUNK_SIZE};
pub use cipher::{ChaChaCipher, Cipher, CipherError, CipherId, NoCipher, StickKey};
pub use manifest::{Entry, Manifest, ManifestError, MANIFEST_FORMAT_VERSION};
pub use record::{ManifestHint, ManifestRecord, RecordError, SignedManifestRecord};
pub use secrets::{check_secret_store, SecretError, SecretName, SecretStore};
pub use stick_id::{InvalidPublicKey, StickId, StickSigningKey};
pub use ticket::{AddressHint, NodeId, Ticket, TicketError};
