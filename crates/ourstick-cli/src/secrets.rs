//! Where this device keeps its keys: the Secret Service, or a key file the
//! user asked for with `--key-file <path>`. The choice is made once, on first
//! use, and recorded in the database; the node key must never change after
//! that (CLAUDE.md rule 9), so the choice cannot change either.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ourstick_core::secrets::{KeyFileParams, KeyFileStore, SecretStore};
use ourstick_core::SecretBackend;

/// Shortest passphrase accepted for a new key file.
const MIN_PASSPHRASE_CHARS: usize = 8;

/// What the user asked for on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretsChoice {
    /// The Secret Service, unless the device already uses a key file.
    Default,
    /// `--key-file <path>`.
    KeyFile(PathBuf),
}

/// Asks for passphrases. On the terminal in the CLI; fixed in tests.
pub trait Passphrases {
    /// The passphrase of the existing key file at `path`.
    fn existing(&self, path: &Path) -> Result<String>;
    /// A passphrase for a new key file at `path`, asked twice.
    fn new_one(&self, path: &Path) -> Result<(String, String)>;
}

/// Reads passphrases from the terminal without echo. Without a terminal it
/// fails; a passphrase never comes from arguments or the environment.
pub struct TerminalPassphrases;

fn prompt(text: &str) -> Result<String> {
    rpassword::prompt_password(text)
        .context("a passphrase is needed, but there is no terminal to ask for it")
}

impl Passphrases for TerminalPassphrases {
    fn existing(&self, path: &Path) -> Result<String> {
        prompt(&format!("Passphrase for {}: ", path.display()))
    }

    fn new_one(&self, path: &Path) -> Result<(String, String)> {
        println!(
            "Creating the key file {}. Its passphrase cannot be recovered: \
             without it, every ticket you share from this device stops working.",
            path.display()
        );
        let first = prompt(&format!(
            "New passphrase (at least {MIN_PASSPHRASE_CHARS} characters): "
        ))?;
        let second = prompt("Repeat the passphrase: ")?;
        Ok((first, second))
    }
}

/// This system's keyring: the Secret Service on Linux, the keychain on macOS,
/// the Credential Manager on Windows.
pub struct SystemKeyring<'a> {
    /// `None` where OurStick has no keyring backend.
    pub backend: Option<SecretBackend>,
    pub open: &'a dyn Fn() -> Result<Box<dyn SecretStore>>,
}

/// A backend's name, for messages.
fn backend_name(backend: &SecretBackend) -> String {
    match backend {
        SecretBackend::SecretService => "the Secret Service".to_owned(),
        SecretBackend::Keychain => "the macOS keychain".to_owned(),
        SecretBackend::CredentialManager => "the Windows Credential Manager".to_owned(),
        SecretBackend::KeyFile(path) => path.display().to_string(),
    }
}

/// The secret store to use, and the backend to record for a new device.
///
/// `recorded` is the backend in the database (`None` for a new device).
pub fn open_secrets(
    recorded: Option<&SecretBackend>,
    choice: &SecretsChoice,
    system: &SystemKeyring<'_>,
    passphrases: &dyn Passphrases,
    params: KeyFileParams,
) -> Result<(Box<dyn SecretStore>, SecretBackend)> {
    match (recorded, choice) {
        (Some(SecretBackend::KeyFile(recorded)), choice) => {
            if let SecretsChoice::KeyFile(given) = choice {
                if absolute(given)? != *recorded {
                    bail!(
                        "this device keeps its keys in {}, not in {}; run with that \
                         path, or without --key-file",
                        recorded.display(),
                        given.display()
                    );
                }
            }
            let passphrase = passphrases.existing(recorded)?;
            let store = KeyFileStore::open(recorded, &passphrase)
                .with_context(|| format!("cannot open the key file {}", recorded.display()))?;
            Ok((Box::new(store), SecretBackend::KeyFile(recorded.clone())))
        }
        (Some(backend), SecretsChoice::KeyFile(_)) => bail!(
            "this device keeps its keys in {}. Moving them to a key file is not \
             supported yet; run without --key-file.",
            backend_name(backend)
        ),
        (Some(backend), SecretsChoice::Default) => {
            if system.backend.as_ref() != Some(backend) {
                bail!(
                    "this device's keys are in {}, which this system does not have \
                     (was the database copied from another computer?)",
                    backend_name(backend)
                );
            }
            Ok(((system.open)()?, backend.clone()))
        }
        (None, SecretsChoice::KeyFile(given)) => {
            let path = absolute(given)?;
            if path.exists() {
                bail!(
                    "{} already exists, but this device has no keys yet. OurStick will \
                     not overwrite it; choose another path, or restore the database that \
                     goes with it.",
                    path.display()
                );
            }
            let (first, second) = passphrases.new_one(&path)?;
            if first != second {
                bail!("the two passphrases differ; nothing was created");
            }
            if first.chars().count() < MIN_PASSPHRASE_CHARS {
                bail!("the passphrase needs at least {MIN_PASSPHRASE_CHARS} characters");
            }
            let store = KeyFileStore::create(&path, &first, params)
                .with_context(|| format!("cannot create the key file {}", path.display()))?;
            Ok((Box::new(store), SecretBackend::KeyFile(path)))
        }
        (None, SecretsChoice::Default) => {
            let no_keyring = "no system keyring to keep the keys in. To keep them in a file \
                 protected by a passphrase instead, run: ourstick share --key-file <path> \
                 <folder>";
            let Some(backend) = system.backend.clone() else {
                bail!("{no_keyring}");
            };
            let store = (system.open)().context(no_keyring)?;
            Ok((store, backend))
        }
    }
}

fn absolute(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("cannot resolve {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ourstick_core::secrets::SecretName;

    /// Cheap Argon2 settings, so the tests are fast.
    const FAST: KeyFileParams = KeyFileParams {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };

    struct Answers {
        existing: &'static str,
        new_one: (&'static str, &'static str),
    }

    impl Passphrases for Answers {
        fn existing(&self, _path: &Path) -> Result<String> {
            Ok(self.existing.to_owned())
        }
        fn new_one(&self, _path: &Path) -> Result<(String, String)> {
            Ok((self.new_one.0.to_owned(), self.new_one.1.to_owned()))
        }
    }

    const GOOD: Answers = Answers {
        existing: "correct horse",
        new_one: ("correct horse", "correct horse"),
    };

    fn no_secret_service() -> Result<Box<dyn SecretStore>> {
        bail!("no Secret Service here")
    }

    fn open(
        recorded: Option<&SecretBackend>,
        choice: &SecretsChoice,
        answers: &Answers,
    ) -> Result<(Box<dyn SecretStore>, SecretBackend)> {
        let system = SystemKeyring {
            backend: Some(SecretBackend::SecretService),
            open: &no_secret_service,
        };
        open_secrets(recorded, choice, &system, answers, FAST)
    }

    #[test]
    fn keys_in_another_systems_keyring_are_not_looked_for_here() {
        let error = open(
            Some(&SecretBackend::Keychain),
            &SecretsChoice::Default,
            &GOOD,
        )
        .err()
        .expect("an error");
        assert!(error.to_string().contains("the macOS keychain"), "{error}");
    }

    #[test]
    fn new_device_with_key_file_creates_it_and_reopens_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.oskf");
        let choice = SecretsChoice::KeyFile(path.clone());

        let (store, backend) = open(None, &choice, &GOOD).unwrap();
        assert_eq!(backend, SecretBackend::KeyFile(path.clone()));
        store.put(&SecretName::NodeKey, &[5; 32]).unwrap();
        drop(store);

        // Later runs: from the database, with or without the flag.
        for choice in [choice, SecretsChoice::Default] {
            let (store, _) = open(Some(&backend), &choice, &GOOD).unwrap();
            assert_eq!(store.get(&SecretName::NodeKey).unwrap(), Some(vec![5; 32]));
        }
    }

    #[test]
    fn wrong_passphrase_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.oskf");
        let (_, backend) = open(None, &SecretsChoice::KeyFile(path), &GOOD).unwrap();
        let wrong = Answers {
            existing: "wrong horse",
            ..GOOD
        };
        let error = open(Some(&backend), &SecretsChoice::Default, &wrong)
            .err()
            .expect("an error");
        assert!(
            format!("{error:#}").contains("wrong passphrase"),
            "{error:#}"
        );
    }

    #[test]
    fn new_passphrase_must_match_and_be_long_enough() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.oskf");
        let choice = SecretsChoice::KeyFile(path.clone());
        for new_one in [("correct horse", "correct hose"), ("short", "short")] {
            let answers = Answers { new_one, ..GOOD };
            assert!(open(None, &choice, &answers).is_err());
            assert!(!path.exists(), "a file was created for {new_one:?}");
        }
    }

    #[test]
    fn existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keys.oskf");
        std::fs::write(&path, b"something else").unwrap();
        let error = open(None, &SecretsChoice::KeyFile(path.clone()), &GOOD)
            .err()
            .expect("an error");
        assert!(error.to_string().contains("will not overwrite"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"something else");
    }

    #[test]
    fn missing_secret_service_suggests_a_key_file() {
        let error = open(None, &SecretsChoice::Default, &GOOD)
            .err()
            .expect("an error");
        assert!(format!("{error:#}").contains("--key-file"), "{error:#}");
    }

    #[test]
    fn the_backend_of_a_device_never_changes() {
        let dir = tempfile::tempdir().unwrap();
        let key_file = SecretsChoice::KeyFile(dir.path().join("keys.oskf"));
        // Secret Service device: a key file is refused.
        assert!(open(Some(&SecretBackend::SecretService), &key_file, &GOOD).is_err());
        // Key-file device: another path is refused, before any passphrase.
        let (_, backend) = open(None, &key_file, &GOOD).unwrap();
        let other = SecretsChoice::KeyFile(dir.path().join("other.oskf"));
        let error = open(Some(&backend), &other, &GOOD).err().expect("an error");
        assert!(error.to_string().contains("not in"), "{error}");
        assert!(!dir.path().join("other.oskf").exists());
    }
}
