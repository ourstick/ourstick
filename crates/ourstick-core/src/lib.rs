//! OurStick core: a USB stick you can send.
//!
//! This crate holds all domain logic and depends on no UI.
//! See CLAUDE.md at the repository root for the architecture rules.

pub mod manifest;

pub use manifest::{Entry, Manifest, ManifestError, MANIFEST_FORMAT_VERSION};
