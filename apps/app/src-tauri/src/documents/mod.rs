//! The sender's files on a phone (ADR 0029): what the user chose with
//! Android's pickers, read where it is through `Documents.kt`; any other
//! path (a stick's own, private folder) from the file system. A chosen
//! place's path carries its content URI ([`paths`]); the URIs of what is
//! inside a chosen folder are kept from its last listing.

pub mod paths;

#[cfg(target_os = "android")]
mod jvm;

#[cfg(target_os = "android")]
pub use android::DocumentFiles;

#[cfg(target_os = "android")]
mod android {
    use std::collections::{HashMap, HashSet};
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use ourstick_core::manifest::FileStamp;
    use ourstick_core::sender_files::{FileFacts, FileKind, LocalFiles, ReadAt, SenderFiles};

    use super::jvm::{Jvm, Told};
    use super::paths::{chosen, is_chosen, is_usable_name};

    /// What the user chose, read through Android.
    #[derive(Default)]
    pub struct DocumentFiles {
        /// Set once the window has started.
        jvm: Arc<OnceLock<Jvm>>,
        /// The URI of each path inside a chosen folder, from its last listing.
        uris: Mutex<HashMap<PathBuf, String>>,
    }

    impl std::fmt::Debug for DocumentFiles {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("DocumentFiles")
        }
    }

    impl DocumentFiles {
        /// Reads through `jvm` from now on.
        pub fn ready(&self, jvm: Jvm) {
            let _ = self.jvm.set(jvm);
        }

        fn jvm(&self) -> io::Result<&Jvm> {
            ready(&self.jvm)
        }

        /// The URI of the chosen place or path inside one at `path`: a
        /// place's from its path, others from their folder's listing (read
        /// again if not known, e.g. after a restart).
        fn uri_of(&self, path: &Path) -> io::Result<String> {
            let found = chosen(path).ok_or(io::ErrorKind::NotFound)?;
            if found.is_place {
                return Ok(found.uri);
            }
            if let Some(uri) = lock(&self.uris).get(path) {
                return Ok(uri.clone());
            }
            let parent = path.parent().ok_or(io::ErrorKind::NotFound)?;
            self.listed(parent)?;
            lock(&self.uris)
                .get(path)
                .cloned()
                .ok_or_else(|| io::ErrorKind::NotFound.into())
        }

        /// The entries of the chosen folder at `path`, their URIs kept.
        fn listed(&self, path: &Path) -> io::Result<Vec<(OsString, FileFacts)>> {
            let uri = self.uri_of(path)?;
            let entries = self.jvm()?.entries(&uri)?;
            let mut names = HashSet::new();
            let mut listed = Vec::with_capacity(entries.len());
            let mut uris = lock(&self.uris);
            for entry in entries {
                // A name no path can hold, or one the folder has twice, is
                // left out: never a manifest path twice.
                if !is_usable_name(&entry.name) || !names.insert(entry.name.clone()) {
                    tracing::debug!("a chosen folder's entry was left out");
                    continue;
                }
                uris.insert(path.join(&entry.name), entry.uri);
                listed.push((OsString::from(entry.name), facts(entry.told)));
            }
            Ok(listed)
        }
    }

    /// The Java VM once the window has started; a few seconds' wait at
    /// most (sticks are shared again as the app starts, from the core's
    /// blocking threads, maybe before the window).
    fn ready(jvm: &OnceLock<Jvm>) -> io::Result<&Jvm> {
        for _ in 0..200 {
            if let Some(jvm) = jvm.get() {
                return Ok(jvm);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Err(io::Error::other("the app has not started yet"))
    }

    fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The facts Android told. A file of untold size is not shared; one of
    /// untold time is read again at every look (its stamp never matches).
    fn facts([kind, size, modified_ms]: Told) -> FileFacts {
        let kind = match kind {
            0 if size >= 0 => FileKind::File,
            1 => FileKind::Folder,
            _ => FileKind::Other,
        };
        let (mtime_ns, ctime_ns) = match u64::try_from(modified_ms) {
            Ok(ms) => (ms.saturating_mul(1_000_000), 0),
            Err(_) => (
                0,
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(1, |now| now.as_nanos() as u64),
            ),
        };
        FileFacts {
            kind,
            stamp: FileStamp {
                size: u64::try_from(size).unwrap_or(0),
                mtime_ns,
                ctime_ns,
            },
            disk: None,
        }
    }

    impl SenderFiles for DocumentFiles {
        fn facts(&self, path: &Path) -> io::Result<FileFacts> {
            if !is_chosen(path) {
                return LocalFiles.facts(path);
            }
            let uri = self.uri_of(path)?;
            Ok(facts(self.jvm()?.facts(&uri)?))
        }

        /// No links among chosen files: as [`SenderFiles::facts`].
        fn facts_followed(&self, path: &Path) -> io::Result<FileFacts> {
            if !is_chosen(path) {
                return LocalFiles.facts_followed(path);
            }
            self.facts(path)
        }

        fn entries(&self, path: &Path) -> io::Result<Vec<(OsString, FileFacts)>> {
            if !is_chosen(path) {
                return LocalFiles.entries(path);
            }
            self.listed(path)
        }

        fn open(&self, path: &Path) -> io::Result<Box<dyn ReadAt>> {
            if !is_chosen(path) {
                return LocalFiles.open(path);
            }
            let uri = self.uri_of(path)?;
            let handle = self.jvm()?.open(&uri)?;
            Ok(Box::new(Opened {
                jvm: self.jvm.clone(),
                handle,
            }))
        }
    }

    /// A chosen file open in `Documents.kt`, closed when dropped.
    struct Opened {
        jvm: Arc<OnceLock<Jvm>>,
        handle: i32,
    }

    impl Opened {
        fn jvm(&self) -> io::Result<&Jvm> {
            ready(&self.jvm)
        }
    }

    impl ReadAt for Opened {
        fn read_at(&self, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
            self.jvm()?.read_at(self.handle, offset, buffer)
        }

        fn facts(&self) -> io::Result<FileFacts> {
            Ok(facts(self.jvm()?.open_facts(self.handle)?))
        }
    }

    impl Drop for Opened {
        fn drop(&mut self) {
            if let Ok(jvm) = self.jvm() {
                if let Err(error) = jvm.close(self.handle) {
                    tracing::debug!(%error, "a chosen file was not closed");
                }
            }
        }
    }

    /// Starts reading through the window's JNI: called once it exists.
    pub fn start<R: tauri::Runtime>(
        window: &tauri::WebviewWindow<R>,
        files: Arc<DocumentFiles>,
    ) -> tauri::Result<()> {
        window.with_webview(move |webview| {
            webview
                .jni_handle()
                .exec(move |env, activity, _| match Jvm::new(env, activity) {
                    Ok(jvm) => files.ready(jvm),
                    Err(error) => tracing::warn!(%error, "chosen files cannot be read"),
                });
        })
    }
}

#[cfg(target_os = "android")]
pub use android::start;
