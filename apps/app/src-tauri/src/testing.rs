//! For the app's tests: a fresh folder in the system's temporary folder,
//! removed when dropped (the app has no `tempfile`).

use std::path::{Path, PathBuf};

pub struct TempFolder(PathBuf);

impl TempFolder {
    pub fn new() -> Self {
        let name: String = ourstick_core::device::random_key()[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let path = std::env::temp_dir().join(format!("ourstick-test-{name}"));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
