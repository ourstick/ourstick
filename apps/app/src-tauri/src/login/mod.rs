//! Start at login (ADR 0024): the system's own entry that starts the app
//! when the user logs in, with `--autostarted` (no window). Windows: a
//! value in `HKCU\…\Run`; macOS: a LaunchAgent; Linux: an XDG autostart
//! entry. What each holds, who may write it and what to do with it are
//! decided here, without the system; each platform only reads and writes.
//!
//! The choice is the store's (`settings.start_at_login`). Only the installed
//! app writes the entry; the system's own "off" (Task Manager, the desktop's
//! startup settings) is never undone at a start, only by the user turning
//! the choice on again in OurStick.

use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::no_relaunch_at_login;

pub mod entries;

/// The argument the login entry starts the app with.
pub const AUTOSTARTED: &str = "--autostarted";

/// Whether this start was the login entry's.
pub fn autostarted<S: AsRef<str>>(args: &[S]) -> bool {
    args.iter().skip(1).any(|arg| arg.as_ref() == AUTOSTARTED)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    Windows,
    MacOs,
    Linux,
}

/// Where this program runs from, and how it was built or started: what
/// decides whether it may write the login entry.
#[derive(Debug, Clone, Copy)]
pub struct Program<'a> {
    pub system: System,
    pub exe: &'a Path,
    pub home: Option<&'a Path>,
    pub debug: bool,
    /// `OURSTICK_DATA_DIR` is set: the entry would start the default data
    /// folder, not this one.
    pub own_data_dir: bool,
    /// Run from an AppImage (not shipped): its path is a temporary mount.
    pub appimage: bool,
}

/// Whether `program` is the installed app, the only one that writes or
/// removes the login entry: a release build, the default data folder, run
/// from where the installer puts it.
pub fn may_write(program: &Program) -> bool {
    if program.debug || program.own_data_dir || program.appimage {
        return false;
    }
    let exe = program.exe;
    match program.system {
        // Per user or per machine, wherever the user chose; never a build.
        // (Split by text, so that a Windows path reads the same anywhere.)
        System::Windows => !exe
            .to_string_lossy()
            .split(['\\', '/'])
            .any(|part| part.eq_ignore_ascii_case("target")),
        // Inside an app under /Applications or ~/Applications: not a disk
        // image, not an App Translocation copy.
        System::MacOs => {
            let text = exe.to_string_lossy();
            let applications = Path::new("/Applications");
            let own = program.home.map(|home| home.join("Applications"));
            let placed =
                exe.starts_with(applications) || own.is_some_and(|own| exe.starts_with(own));
            placed && text.contains(".app/Contents/MacOS/") && !text.contains("/AppTranslocation/")
        }
        System::Linux => exe == Path::new("/usr/bin/ourstick-app"),
    }
}

/// The system's entry as it is now, if there is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Found {
    /// It starts this program.
    pub ours: bool,
    /// The user turned it off in the system's own settings.
    pub system_off: bool,
}

/// What to do with the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Leave,
    /// Write it; `clear_off` also undoes the system's own "off".
    Write {
        clear_off: bool,
    },
    Remove,
}

/// At a start: the entry made to match the choice `on`, written only when
/// it differs, and the system's own "off" kept.
pub fn at_start(on: bool, found: Option<Found>) -> Step {
    match (on, found) {
        (true, None) => Step::Write { clear_off: false },
        (true, Some(found)) if found.system_off || found.ours => Step::Leave,
        (true, Some(_)) => Step::Write { clear_off: false },
        (false, None) => Step::Leave,
        (false, Some(_)) => Step::Remove,
    }
}

/// The user turned the choice on or off in OurStick: on undoes the
/// system's own "off" too.
pub fn by_user(on: bool) -> Step {
    if on {
        Step::Write { clear_off: true }
    } else {
        Step::Remove
    }
}

/// What the check shows: the choice, off if the system turned it off.
pub fn shown_on(on: bool, found: Option<Found>) -> bool {
    on && !found.is_some_and(|found| found.system_off)
}

/// How long a read of the system's entry is used again: the tray asks at
/// each change of a stick, and on Windows each read starts `reg` twice. The
/// system's own "off" shows in the check this late.
pub const READ_AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);

/// The last read of the system's entry, and when.
#[derive(Debug, Default)]
pub struct Kept(Option<(Instant, Option<Found>)>);

impl Kept {
    pub const fn new() -> Self {
        Self(None)
    }

    /// The entry: the kept read if younger than [`READ_AGAIN_AFTER`] at
    /// `now`, else `read` again.
    pub fn get(&mut self, now: Instant, read: impl FnOnce() -> Option<Found>) -> Option<Found> {
        match self.0 {
            Some((at, found)) if now.saturating_duration_since(at) < READ_AGAIN_AFTER => found,
            _ => {
                let found = read();
                self.0 = Some((now, found));
                found
            }
        }
    }

    /// The entry was written or removed: the next ask reads it.
    pub fn forget(&mut self) {
        self.0 = None;
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod run {
    use super::*;
    use anyhow::{Context, Result};
    use ourstick_core::store::{Settings, Store};

    #[cfg(target_os = "linux")]
    use super::linux as platform;
    #[cfg(target_os = "macos")]
    use super::macos as platform;
    #[cfg(windows)]
    use super::windows as platform;

    const SYSTEM: System = if cfg!(windows) {
        System::Windows
    } else if cfg!(target_os = "macos") {
        System::MacOs
    } else {
        System::Linux
    };

    /// This program, if it may write the entry.
    fn writer() -> Option<std::path::PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        may_write(&Program {
            system: SYSTEM,
            exe: &exe,
            home: home.as_deref(),
            debug: cfg!(debug_assertions),
            own_data_dir: set("OURSTICK_DATA_DIR"),
            appimage: set("APPIMAGE"),
        })
        .then_some(exe)
    }

    fn apply(step: Step, exe: &Path) -> Result<()> {
        match step {
            Step::Leave => Ok(()),
            Step::Write { clear_off } => platform::write(exe, clear_off),
            Step::Remove => platform::remove(),
        }
    }

    /// At a start: the entry made to match the store's choice. Logged,
    /// never stopping the start.
    pub fn sync_at_start(store: &dyn Store) {
        let Some(exe) = writer() else {
            return;
        };
        let on = store.settings().unwrap_or_default().start_at_login;
        let step = at_start(on, platform::read(&exe));
        let applied = apply(step, &exe);
        kept().forget();
        if let Err(error) = applied {
            tracing::warn!(error = %format!("{error:#}"), "the login entry was not made to match");
        }
    }

    /// The user's choice, from the tray or About: stored, then the entry
    /// (for the installed app).
    pub fn set_by_user(store: &dyn Store, on: bool) -> Result<()> {
        let settings = store.settings()?;
        store.set_settings(&Settings {
            start_at_login: on,
            ..settings
        })?;
        if let Some(exe) = writer() {
            let applied = apply(by_user(on), &exe);
            kept().forget();
            applied.context("cannot change the login entry")?;
        }
        Ok(())
    }

    /// What the check shows now.
    pub fn shown(store: &dyn Store) -> bool {
        let on = store.settings().unwrap_or_default().start_at_login;
        if !on {
            return false;
        }
        let found = kept().get(Instant::now(), || {
            std::env::current_exe()
                .ok()
                .and_then(|exe| platform::read(&exe))
        });
        shown_on(on, found)
    }

    /// The last read of the system's entry, shared by the tray and About.
    fn kept() -> std::sync::MutexGuard<'static, Kept> {
        static KEPT: std::sync::Mutex<Kept> = std::sync::Mutex::new(Kept::new());
        KEPT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub use run::{set_by_user, shown, sync_at_start};

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub mod commands;

#[cfg(test)]
mod tests;
