//! A received file is marked as one from the internet (ADR 0032), as a
//! browser marks a download: Windows then has SmartScreen and Office's
//! Protected View look at it before it runs or opens, and macOS has
//! Gatekeeper. Best effort: a disk that cannot hold the mark (FAT, exFAT)
//! keeps the file without it.

use std::path::Path;

/// Marks `file`, before it is moved into place. On Windows the mark is a
/// stream of the file, and writing it changes the file's time: so this
/// comes before the time is set.
pub(super) fn mark(file: &Path) {
    #[cfg(windows)]
    windows::mark(file);
    #[cfg(target_os = "macos")]
    macos::mark(file);
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = file;
}

#[cfg(windows)]
mod windows {
    use std::path::Path;

    /// The Mark of the Web: zone 3, the internet.
    pub(super) const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

    pub(super) fn mark(file: &Path) {
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        if let Err(error) = std::fs::write(&stream, ZONE_IDENTIFIER) {
            tracing::debug!(%error, "a received file could not be marked as from the internet");
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The quarantine as Safari writes it: flags (a download), the time in
    /// hex, the agent.
    pub(super) fn value(now: SystemTime) -> String {
        let seconds = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        format!("0081;{seconds:x};OurStick;")
    }

    pub(super) fn mark(file: &Path) {
        let set = std::process::Command::new("/usr/bin/xattr")
            .arg("-w")
            .arg("com.apple.quarantine")
            .arg(value(SystemTime::now()))
            .arg(file)
            .output();
        match set {
            Ok(output) if output.status.success() => {}
            Ok(_) => tracing::debug!("a received file could not be put in quarantine"),
            Err(error) => {
                tracing::debug!(%error, "a received file could not be put in quarantine");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_file_keeps_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, b"MZ").unwrap();
        mark(&file);
        assert_eq!(std::fs::read(&file).unwrap(), b"MZ");
        // A file that is not there is no error.
        mark(&dir.path().join("gone"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_the_mark_of_the_web() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, b"MZ").unwrap();
        mark(&file);
        let mut stream = file.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        assert_eq!(
            std::fs::read_to_string(stream).unwrap(),
            windows::ZONE_IDENTIFIER
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_has_the_quarantine() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(
            macos::value(UNIX_EPOCH + Duration::from_secs(0x6000_0000)),
            "0081;60000000;OurStick;"
        );
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool");
        std::fs::write(&file, b"#!").unwrap();
        mark(&file);
        let read = std::process::Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&file)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&read.stdout).starts_with("0081;"));
    }
}
