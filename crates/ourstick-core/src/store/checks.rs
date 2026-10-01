//! What every [`Store`](super::Store) checks before it writes.

use std::path::Path;

use super::*;

/// Largest `seq` a store accepts (SQLite INTEGER is signed 64-bit).
pub(crate) const MAX_STORED_SEQ: u64 = i64::MAX as u64;

pub(crate) fn check_seq(seq: u64) -> Result<(), StoreError> {
    if seq > MAX_STORED_SEQ {
        Err(StoreError::SeqOutOfRange(seq))
    } else {
        Ok(())
    }
}

pub(crate) fn check_time(ms: u64) -> Result<(), StoreError> {
    if ms > MAX_STORED_SEQ {
        Err(StoreError::TimeOutOfRange(ms))
    } else {
        Ok(())
    }
}

pub(crate) fn path_str(path: &Path) -> Result<&str, StoreError> {
    path.to_str()
        .ok_or_else(|| StoreError::NonUtf8Path(path.to_path_buf()))
}

/// Checks the rules every store enforces before writing an owned stick.
/// A drive letter is one of `A`–`Z`.
pub(crate) fn check_drive_letter(letter: Option<char>) -> Result<(), StoreError> {
    match letter {
        Some(letter) if !letter.is_ascii_uppercase() => {
            Err(StoreError::Inconsistent("a drive letter must be A to Z"))
        }
        _ => Ok(()),
    }
}

pub(crate) fn check_owned_stick(stick: &OwnedStick) -> Result<(), StoreError> {
    check_seq(stick.last_signed_seq)?;
    if stick.drive_size.is_some() && stick.kind != StickKind::App {
        return Err(StoreError::Inconsistent("a drive is the app's stick"));
    }
    check_drive_letter(stick.drive_letter)?;
    match stick.drive_size {
        Some(0) => return Err(StoreError::Inconsistent("a drive of size 0")),
        Some(size) if size > MAX_STORED_SEQ => {
            return Err(StoreError::ValueOutOfRange("drive size", size))
        }
        _ => {}
    }
    check_time(stick.created_at_ms)?;
    path_str(&stick.root_path)?;
    match &stick.latest_record {
        None if stick.last_signed_seq != 0 => Err(StoreError::Inconsistent(
            "a stick that has signed must keep its latest record",
        )),
        Some(record) => {
            let content = record.unverified_record();
            if content.stick_id != stick.stick_id {
                Err(StoreError::Inconsistent("record belongs to another stick"))
            } else if content.seq != stick.last_signed_seq {
                Err(StoreError::Inconsistent(
                    "record seq differs from last_signed_seq",
                ))
            } else {
                Ok(())
            }
        }
        None => Ok(()),
    }
}

/// Checks the rules every store enforces before writing a joined stick.
pub(crate) fn check_joined_stick(stick: &JoinedStick) -> Result<(), StoreError> {
    path_str(&stick.output_dir)?;
    check_time(stick.joined_at_ms)?;
    if let Some(completed) = &stick.completed {
        check_seq(completed.seq)?;
    }
    Ok(())
}

/// Checks that every number of a recorded file fits in an SQLite INTEGER.
pub(crate) fn check_owned_file(file: &OwnedFile) -> Result<(), StoreError> {
    check_stamp(&file.stamp)
}

pub(crate) fn check_stamp(stamp: &FileStamp) -> Result<(), StoreError> {
    for (name, value) in [
        ("file size", stamp.size),
        ("mtime", stamp.mtime_ns),
        ("ctime", stamp.ctime_ns),
    ] {
        if value > MAX_STORED_SEQ {
            return Err(StoreError::ValueOutOfRange(name, value));
        }
    }
    Ok(())
}

pub(crate) fn check_device(device: &Device) -> Result<(), StoreError> {
    if let SecretBackend::KeyFile(path) = &device.secret_backend {
        path_str(path)?;
    }
    check_time(device.created_at_ms)
}

/// A linked entry's name is one manifest path component; its path is
/// absolute.
pub(crate) fn check_linked_entry(entry: &LinkedEntry) -> Result<(), StoreError> {
    let name_ok = !entry.name.is_empty()
        && !matches!(entry.name.as_str(), "." | "..")
        && !entry.name.contains(['/', '\\', '\0']);
    if !name_ok {
        return Err(StoreError::Inconsistent(
            "a linked entry's name is one path component",
        ));
    }
    if !entry.path.is_absolute() {
        return Err(StoreError::Inconsistent(
            "a linked entry's path is absolute",
        ));
    }
    path_str(&entry.path)?;
    check_time(entry.added_at_ms)
}

/// A count of bytes or the like that SQLite's signed 64-bit integer holds.
pub(crate) fn check_value(name: &'static str, value: u64) -> Result<(), StoreError> {
    if value > MAX_STORED_SEQ {
        Err(StoreError::ValueOutOfRange(name, value))
    } else {
        Ok(())
    }
}
