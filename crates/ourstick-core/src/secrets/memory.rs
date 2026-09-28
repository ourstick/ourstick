//! Secrets in process memory, for tests. Not durable, so it never passes
//! [`check_secret_store`](super::check_secret_store).

use std::collections::HashMap;
use std::sync::Mutex;

use super::{Durability, SecretError, SecretName, SecretStore};

/// A [`SecretStore`] that forgets everything when dropped.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    secrets: Mutex<HashMap<SecretName, Vec<u8>>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn secrets(&self) -> std::sync::MutexGuard<'_, HashMap<SecretName, Vec<u8>>> {
        // A panic while holding the lock cannot leave the map half-updated,
        // since every operation is a single map call.
        self.secrets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl SecretStore for MemorySecretStore {
    fn describe(&self) -> String {
        "memory (for tests)".to_owned()
    }

    fn durability(&self) -> Durability {
        Durability::Ephemeral
    }

    fn put(&self, name: &SecretName, secret: &[u8]) -> Result<(), SecretError> {
        self.secrets().insert(name.clone(), secret.to_vec());
        Ok(())
    }

    fn get(&self, name: &SecretName) -> Result<Option<Vec<u8>>, SecretError> {
        Ok(self.secrets().get(name).cloned())
    }

    fn delete(&self, name: &SecretName) -> Result<(), SecretError> {
        self.secrets().remove(name);
        Ok(())
    }
}
