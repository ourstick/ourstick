//! Wire format of a manifest: one format-version byte, then the postcard encoding.
//! Version 2 (written) starts with the `partial` mark and the stick's name;
//! version 1 (still read) has only the entries.
//!
//! Decoding treats its input as untrusted: it never panics, and it rejects any
//! manifest a reader could not safely write to disk. See `docs/protocol.md`.

use serde::Deserialize;

use super::{Entry, Manifest, ManifestError, MAX_NAME_CHARS};

/// Version of the manifest wire format written by [`Manifest::to_bytes`].
pub const MANIFEST_FORMAT_VERSION: u8 = 2;

/// Version 1: the entries alone.
#[derive(Deserialize)]
struct ManifestV1 {
    entries: Vec<Entry>,
}

impl Manifest {
    /// Encodes the manifest into its wire format.
    ///
    /// Fails if the manifest is not valid (see [`Manifest`]), so a peer never
    /// sends a manifest that other peers would reject.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        self.validate()?;
        let mut bytes = vec![MANIFEST_FORMAT_VERSION];
        bytes.extend(postcard::to_allocvec(self)?);
        Ok(bytes)
    }

    /// Decodes a manifest from its wire format and validates it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let (&version, body) = bytes.split_first().ok_or(ManifestError::EmptyInput)?;
        let (manifest, rest) = match version {
            2 => postcard::take_from_bytes::<Self>(body)?,
            1 => {
                let (old, rest) = postcard::take_from_bytes::<ManifestV1>(body)?;
                let manifest = Self {
                    entries: old.entries,
                    ..Self::default()
                };
                (manifest, rest)
            }
            other => return Err(ManifestError::UnsupportedVersion(other)),
        };
        if !rest.is_empty() {
            return Err(ManifestError::TrailingBytes(rest.len()));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks the name, that every path is safe, and that entries are strictly
    /// sorted by path.
    fn validate(&self) -> Result<(), ManifestError> {
        validate_name(&self.name)?;
        for entry in &self.entries {
            validate_path(&entry.path)?;
        }
        for pair in self.entries.windows(2) {
            if pair[0].path >= pair[1].path {
                return Err(ManifestError::NotSorted(pair[1].path.clone()));
            }
        }
        Ok(())
    }
}

/// Whether `c` is a format character (Unicode category Cf), or a line or
/// paragraph separator (Zl, Zp): invisible, or able to reorder or break text
/// shown to a user.
fn is_format_or_separator(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}' | '\u{070F}'
        | '\u{0890}'..='\u{0891}' | '\u{08E2}' | '\u{180E}' | '\u{200B}'..='\u{200F}'
        | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206F}'
        | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{110BD}' | '\u{110CD}'
        | '\u{13430}'..='\u{1343F}' | '\u{1BCA0}'..='\u{1BCA3}'
        | '\u{1D173}'..='\u{1D17A}' | '\u{E0001}' | '\u{E0020}'..='\u{E007F}')
}

/// Checks a stick's name as it travels: at most [`MAX_NAME_CHARS`] scalar
/// values, none a control, format or separator character (Cc, Cf, Zl, Zp).
pub fn validate_name(name: &str) -> Result<(), ManifestError> {
    let too_long = name.chars().count() > MAX_NAME_CHARS;
    if too_long
        || name
            .chars()
            .any(|c| c.is_control() || is_format_or_separator(c))
    {
        Err(ManifestError::InvalidName(name.to_owned()))
    } else {
        Ok(())
    }
}

/// A stick's name as it goes into its manifest: without the characters
/// [`validate_name`] refuses, trimmed, cut to [`MAX_NAME_CHARS`] scalar
/// values. Always passes [`validate_name`].
pub fn name_for_manifest(name: &str) -> String {
    let kept: String = name
        .chars()
        .filter(|c| !c.is_control() && !is_format_or_separator(*c))
        .collect();
    kept.trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Checks that `path` is a relative, `/`-separated path that stays inside the
/// stick root on every platform.
pub(super) fn validate_path(path: &str) -> Result<(), ManifestError> {
    let is_valid = !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."));
    if is_valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidPath(path.to_owned()))
    }
}

#[cfg(test)]
mod tests;
