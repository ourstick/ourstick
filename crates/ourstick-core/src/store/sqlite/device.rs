//! This device's row (`device`), and the app's choices (`settings`).

use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};

use super::super::{check_device, path_str, Device, SecretBackend, Settings, StoreError};
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
        ("protected-keychain", None) => SecretBackend::ProtectedKeychain,
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
        SecretBackend::ProtectedKeychain => ("protected-keychain", None),
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

/// A 0/1 column of the device row; false before the device is set up.
pub(super) fn flag(connection: &Connection, column: &'static str) -> Result<bool, StoreError> {
    Ok(connection
        .query_row(&format!("SELECT {column} FROM device"), [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()?
        == Some(1))
}

pub(super) fn set_local_network_explained(connection: &Connection) -> Result<bool, StoreError> {
    Ok(connection.execute("UPDATE device SET local_network_explained = 1", [])? > 0)
}

pub(super) fn record_keys_moved(
    connection: &Connection,
    old_keys_left: bool,
) -> Result<(), StoreError> {
    let updated = connection.execute(
        "UPDATE device SET secret_backend = 'protected-keychain', old_keys_left = ?1
         WHERE secret_backend IN ('keychain', 'protected-keychain')",
        [old_keys_left],
    )?;
    if updated == 0 {
        return Err(StoreError::Inconsistent(
            "the device's keys are not in the macOS keychain",
        ));
    }
    Ok(())
}

/// The one row of `settings`, or the defaults.
pub(super) fn settings(connection: &Connection) -> Result<Settings, StoreError> {
    Ok(connection
        .query_row(
            "SELECT start_at_login, start_at_login_told, still_open_told FROM settings WHERE id = 1",
            [],
            |row| {
                Ok(Settings {
                    start_at_login: row.get::<_, i64>(0)? == 1,
                    start_at_login_told: row.get::<_, i64>(1)? == 1,
                    still_open_told: row.get::<_, i64>(2)? == 1,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

pub(super) fn set_settings(connection: &Connection, settings: &Settings) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO settings (id, start_at_login, start_at_login_told, still_open_told)
         VALUES (1, ?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE
         SET start_at_login = excluded.start_at_login,
             start_at_login_told = excluded.start_at_login_told,
             still_open_told = excluded.still_open_told",
        params![
            settings.start_at_login,
            settings.start_at_login_told,
            settings.still_open_told
        ],
    )?;
    Ok(())
}
