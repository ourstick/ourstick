//! Secrets in a file encrypted with a passphrase, for machines without a Secret
//! Service (SSH sessions, servers). Only used when the user asks for it.
//!
//! Layout (see "Key storage" in `docs/protocol.md`):
//!
//! ```text
//! "OSKF" || version (u8) || memory KiB (u32 BE) || iterations (u32 BE)
//!        || parallelism (u32 BE) || salt (16) || nonce (24) || ciphertext
//! ```
//!
//! The key is Argon2id(passphrase, salt). The ciphertext is XChaCha20-Poly1305
//! over `postcard(map of name -> secret)`, with everything before it as AAD.
//! The whole file is rewritten atomically on every change, from what was read
//! when it was opened. So only one program may use a key file at a time, from
//! opening it to its end: a program takes `crate::locks::key_file_lock` before
//! opening it, or two writers would lose each other's changes.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;

use super::{Durability, SecretError, SecretName, SecretStore};

const MAGIC: &[u8; 4] = b"OSKF";
const KEY_FILE_VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 1 + 4 + 4 + 4 + SALT_LEN + NONCE_LEN;
/// Largest key file read, so a wrong path cannot make us read a huge file.
const MAX_FILE_LEN: u64 = 16 * 1024 * 1024;

/// Argon2id cost parameters, stored in the file so they can change later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyFileParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KeyFileParams {
    /// RFC 9106's second recommended option: 64 MiB, 3 passes, 4 lanes.
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 4,
        }
    }
}

impl KeyFileParams {
    /// Bounds accepted when reading a file, so a damaged or hostile header
    /// cannot ask for gigabytes of memory or hours of work.
    fn argon2(&self) -> Result<Argon2<'static>, SecretError> {
        let invalid = |what: &str| SecretError::InvalidKeyFileParams(what.to_owned());
        if !(1..=16).contains(&self.parallelism) {
            return Err(invalid("parallelism must be 1..=16"));
        }
        if !(1..=16).contains(&self.iterations) {
            return Err(invalid("iterations must be 1..=16"));
        }
        if self.memory_kib < 8 * self.parallelism || self.memory_kib > 1024 * 1024 {
            return Err(invalid("memory must be 8 KiB per lane up to 1 GiB"));
        }
        let params = Params::new(self.memory_kib, self.iterations, self.parallelism, Some(32))
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
    }
}

/// A [`SecretStore`] in a passphrase-encrypted file.
pub struct KeyFileStore {
    path: PathBuf,
    params: KeyFileParams,
    salt: [u8; SALT_LEN],
    key: [u8; 32],
    secrets: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl fmt::Debug for KeyFileStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KeyFileStore")
            .field("path", &self.path)
            .field("params", &self.params)
            .finish_non_exhaustive()
    }
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> SecretError + '_ {
    move |source| SecretError::Io {
        path: path.display().to_string(),
        source,
    }
}

fn derive_key(
    params: &KeyFileParams,
    passphrase: &str,
    salt: &[u8; SALT_LEN],
) -> Result<[u8; 32], SecretError> {
    if passphrase.is_empty() {
        return Err(SecretError::EmptyPassphrase);
    }
    let mut key = [0u8; 32];
    params
        .argon2()?
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| SecretError::InvalidKeyFileParams(error.to_string()))?;
    Ok(key)
}

impl KeyFileStore {
    /// Creates a new, empty key file at `path`. Never overwrites an existing file.
    pub fn create(
        path: impl Into<PathBuf>,
        passphrase: &str,
        params: KeyFileParams,
    ) -> Result<Self, SecretError> {
        let path = path.into();
        let mut salt = [0u8; SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let store = Self {
            key: derive_key(&params, passphrase, &salt)?,
            path,
            params,
            salt,
            secrets: Mutex::new(BTreeMap::new()),
        };
        let bytes = store.encode(&BTreeMap::new())?;
        write_new_file(&store.path, &bytes)?;
        Ok(store)
    }

    /// Opens an existing key file. Refuses files that other users can access.
    pub fn open(path: impl Into<PathBuf>, passphrase: &str) -> Result<Self, SecretError> {
        let path = path.into();
        check_permissions(&path)?;
        let mut bytes = Vec::new();
        File::open(&path)
            .map_err(io_error(&path))?
            .take(MAX_FILE_LEN + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error(&path))?;
        let not_a_key_file = || SecretError::NotAKeyFile {
            path: path.display().to_string(),
        };
        if bytes.len() < HEADER_LEN || bytes.len() as u64 > MAX_FILE_LEN || &bytes[..4] != MAGIC {
            return Err(not_a_key_file());
        }
        if bytes[4] != KEY_FILE_VERSION {
            return Err(SecretError::UnsupportedKeyFileVersion {
                path: path.display().to_string(),
                version: bytes[4],
            });
        }
        let u32_at = |offset: usize| {
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4 bytes"))
        };
        let params = KeyFileParams {
            memory_kib: u32_at(5),
            iterations: u32_at(9),
            parallelism: u32_at(13),
        };
        let salt: [u8; SALT_LEN] = bytes[17..33].try_into().expect("16 bytes");
        let key = derive_key(&params, passphrase, &salt)?;
        let secrets = decrypt(&key, &bytes)?;
        Ok(Self {
            path,
            params,
            salt,
            key,
            secrets: Mutex::new(secrets),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn header(&self, nonce: &[u8; NONCE_LEN]) -> Vec<u8> {
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.push(KEY_FILE_VERSION);
        header.extend_from_slice(&self.params.memory_kib.to_be_bytes());
        header.extend_from_slice(&self.params.iterations.to_be_bytes());
        header.extend_from_slice(&self.params.parallelism.to_be_bytes());
        header.extend_from_slice(&self.salt);
        header.extend_from_slice(nonce);
        header
    }

    fn encode(&self, secrets: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, SecretError> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let header = self.header(&nonce);
        let plaintext = postcard::to_allocvec(secrets).expect("encoding to a Vec cannot fail");
        let ciphertext = XChaCha20Poly1305::new(&self.key.into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &header,
                },
            )
            .expect("encryption of a small map cannot fail");
        Ok([header, ciphertext].concat())
    }

    /// Applies `change` to a copy of the secrets, writes it, and only then keeps
    /// it in memory, so memory never runs ahead of the file.
    fn update(
        &self,
        change: impl FnOnce(&mut BTreeMap<String, Vec<u8>>),
    ) -> Result<(), SecretError> {
        let mut secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut updated = secrets.clone();
        change(&mut updated);
        replace_file(&self.path, &self.encode(&updated)?)?;
        *secrets = updated;
        Ok(())
    }
}

fn decrypt(key: &[u8; 32], bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, SecretError> {
    let (header, ciphertext) = bytes.split_at(HEADER_LEN);
    let nonce = XNonce::from_slice(&header[HEADER_LEN - NONCE_LEN..]);
    let plaintext = XChaCha20Poly1305::new(key.into())
        .decrypt(
            nonce,
            Payload {
                msg: ciphertext,
                aad: header,
            },
        )
        .map_err(|_| SecretError::WrongPassphraseOrDamaged)?;
    postcard::from_bytes(&plaintext).map_err(|_| SecretError::WrongPassphraseOrDamaged)
}

/// A key file is a regular file, not a link to one (nor a pipe, which would
/// block the read), and only its owner may read it. On Unix its owner is
/// also its folder's: a file put there by another user is not read.
#[cfg(unix)]
fn check_permissions(path: &Path) -> Result<(), SecretError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let metadata = fs::symlink_metadata(path).map_err(io_error(path))?;
    if !metadata.is_file() {
        return Err(SecretError::NotAKeyFile {
            path: path.display().to_string(),
        });
    }
    let folder = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let folder = fs::metadata(folder.unwrap_or(Path::new("."))).map_err(io_error(path))?;
    if metadata.uid() != folder.uid() {
        return Err(SecretError::NotYours {
            path: path.display().to_string(),
        });
    }
    let mode = metadata.permissions().mode();
    if mode & 0o077 != 0 {
        return Err(SecretError::InsecurePermissions {
            path: path.display().to_string(),
            mode: mode & 0o777,
        });
    }
    Ok(())
}

/// A key file is a regular file, not a link to one. Who may read it is the
/// folder's matter on Windows: the file is encrypted, so one read by another
/// only lets that one guess the passphrase.
#[cfg(not(unix))]
fn check_permissions(path: &Path) -> Result<(), SecretError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error(path))?;
    if !metadata.is_file() {
        return Err(SecretError::NotAKeyFile {
            path: path.display().to_string(),
        });
    }
    Ok(())
}

/// Opens a file for writing that only the owner can read (mode 0600 on Unix).
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let mut file = create_private(path).map_err(io_error(path))?;
    file.write_all(bytes).map_err(io_error(path))?;
    file.sync_all().map_err(io_error(path))?;
    sync_parent(path)
}

/// Replaces `path` atomically: write a private temporary file next to it, sync
/// it, rename it over `path`, and sync the directory.
fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), SecretError> {
    let mut suffix = [0u8; 6];
    rand::rngs::OsRng.fill_bytes(&mut suffix);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", data_encoding::HEXLOWER.encode(&suffix)));
    let temporary = PathBuf::from(temporary);

    let result = write_new_file(&temporary, bytes)
        .and_then(|()| fs::rename(&temporary, path).map_err(io_error(path)))
        .and_then(|()| sync_parent(path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(path: &Path) -> Result<(), SecretError> {
    #[cfg(unix)]
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(io_error(parent))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

impl SecretStore for KeyFileStore {
    fn describe(&self) -> String {
        format!("key file {}", self.path.display())
    }

    fn durability(&self) -> Durability {
        Durability::Durable
    }

    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        self.update(|secrets| {
            secrets.insert(name.as_key(), secret.to_vec());
        })
    }

    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        let secrets = self
            .secrets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Ok(secrets.get(&name.as_key()).cloned())
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        if self.get(name)?.is_none() {
            return Ok(());
        }
        self.update(|secrets| {
            secrets.remove(&name.as_key());
        })
    }
}
