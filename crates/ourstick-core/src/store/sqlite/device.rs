//! This device's row (`device`).

use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{check_device, path_str, Device, SecretBackend, StoreError};
use super::rows::{blob32, is_constraint_violation, time_i64, to_u64};
use crate::ticket::NodeId;

pub(super) fn device(connection: &Connection) -> Result<Option<Device>, StoreError> {
    let row = connection
        .query_row(
            "SELECT node_id, secret_backend, key_file_path, created_at_ms FROM device",
            [],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((node_id, backend, key_file_path, created_at_ms)) = row else {
        return Ok(None);
    };
    let secret_backend = match (backend.as_str(), key_file_path) {
        ("secret-service", None) => SecretBackend::SecretService,
        ("keychain", None) => SecretBackend::Keychain,
        ("credential-manager", None) => SecretBackend::CredentialManager,
        ("key-file", Some(path)) => SecretBackend::KeyFile(PathBuf::from(path)),
        _ => return Err(StoreError::Corrupt(format!("secret_backend {backend}"))),
    };
    Ok(Some(Device {
        node_id: node_id
            .map(|node_id| blob32(node_id, "node_id").map(NodeId))
            .transpose()?,
        secret_backend,
        created_at_ms: to_u64(created_at_ms, "created_at_ms")?,
    }))
}

pub(super) fn create_device(connection: &Connection, device: &Device) -> Result<(), StoreError> {
    check_device(device)?;
    let (backend, key_file_path) = match &device.secret_backend {
        SecretBackend::SecretService => ("secret-service", None),
        SecretBackend::Keychain => ("keychain", None),
        SecretBackend::CredentialManager => ("credential-manager", None),
        SecretBackend::KeyFile(path) => ("key-file", Some(path_str(path)?)),
    };
    let inserted = connection.execute(
        "INSERT INTO device (id, node_id, secret_backend, key_file_path, created_at_ms)
         VALUES (1, ?1, ?2, ?3, ?4)",
        params![
            device.node_id.map(|node_id| node_id.0.to_vec()),
            backend,
            key_file_path,
            time_i64(device.created_at_ms)?
        ],
    );
    match inserted {
        Ok(_) => Ok(()),
        Err(error) if is_constraint_violation(&error) => Err(StoreError::DeviceExists),
        Err(error) => Err(error.into()),
    }
}
