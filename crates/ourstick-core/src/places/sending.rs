//! What may be sent (ADR 0028 §2): the one place that decides, before
//! anything is linked. Refused: the home folder and what holds it, private
//! folders, the system's folders, OurStick's own, a shortcut, and what is,
//! holds or lies inside something in the list already. Asked about first:
//! a main folder of the home, a whole disk. Blocking I/O.

use std::path::{Path, PathBuf};

use super::{comparable, relation, Relation, Taken};

/// What [`may_send`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaySend {
    Fine,
    /// Sent only if the user says so.
    Ask(Ask),
    /// Never, and why, for users.
    Refuse(String),
}

/// What to ask before sending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Documents, Pictures…: they often hold private papers.
    MainFolder { name: String },
    /// A disk that does not hold the home: it can take days.
    WholeDisk { name: String },
}

impl Ask {
    pub fn question(&self) -> String {
        match self {
            Self::MainFolder { name } => format!("Send all of {}?", quoted(name)),
            Self::WholeDisk { name } => format!("Send the whole disk {}?", quoted(name)),
        }
    }
}

/// The places [`may_send`] knows of. Paths as written; they are compared
/// [`comparable`].
#[derive(Debug, Clone, Default)]
pub struct SendRules {
    pub home: PathBuf,
    /// Private folders besides the home's dot-folders (`~/Library`,
    /// `AppData`).
    pub private: Vec<PathBuf>,
    /// The system's folders, and the folders in them that are not the
    /// system's (`/run/media`, where Linux puts disks).
    pub system: Vec<PathBuf>,
    pub not_system: Vec<PathBuf>,
    /// The home's main folders, by the system's own list (so by their
    /// localized names too).
    pub main_folders: Vec<PathBuf>,
    /// OurStick's own places ([`super::app_places`]).
    pub own: Vec<Taken>,
    /// What is in the list already.
    pub chosen: Vec<PathBuf>,
}

impl SendRules {
    /// The rules of this device for the home `home`, with nothing chosen yet.
    pub fn of_this_device(home: &Path, main_folders: Vec<PathBuf>, own: Vec<Taken>) -> Self {
        let (private, system, not_system) = this_system(home);
        Self {
            home: home.to_path_buf(),
            private,
            system,
            not_system,
            main_folders,
            own,
            chosen: Vec::new(),
        }
    }
}

type Places = Vec<PathBuf>;

#[cfg(target_os = "macos")]
fn this_system(home: &Path) -> (Places, Places, Places) {
    let system = [
        "/System",
        "/Library",
        "/Applications",
        "/private",
        "/usr",
        "/bin",
        "/sbin",
        "/etc",
        "/var",
    ];
    (
        vec![home.join("Library")],
        system.iter().map(PathBuf::from).collect(),
        Vec::new(),
    )
}

#[cfg(windows)]
fn this_system(home: &Path) -> (Places, Places, Places) {
    let mut system: Places = [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ]
    .iter()
    .filter_map(std::env::var_os)
    .map(PathBuf::from)
    .collect();
    if system.is_empty() {
        system.push(PathBuf::from(r"C:\Windows"));
    }
    (vec![home.join("AppData")], system, Vec::new())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn this_system(_home: &Path) -> (Places, Places, Places) {
    let system = [
        "/usr", "/etc", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/boot", "/dev",
        "/proc", "/sys", "/var", "/root", "/run", "/opt", "/snap",
    ];
    (
        Vec::new(),
        system.iter().map(PathBuf::from).collect(),
        vec![PathBuf::from("/run/media")],
    )
}

/// Folders of the system at any disk's top (Windows).
const SYSTEM_NAMES: [&str; 2] = ["$recycle.bin", "system volume information"];

/// Whether `path` may be sent, by `rules`.
pub fn may_send(path: &Path, rules: &SendRules) -> MaySend {
    let name = name_of(path);
    if !path.is_absolute() {
        return refuse(format!("{} is not a full path.", quoted(&name)));
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(_) => return refuse(format!("{} is not there any more.", quoted(&name))),
    };
    // On Windows a junction is a symbolic link too (a name surrogate);
    // OneDrive's placeholders are not.
    if metadata.file_type().is_symlink() {
        return refuse(format!(
            "{} is a shortcut. Choose what it points to instead.",
            quoted(&name)
        ));
    }
    let here = comparable(path);
    let home = comparable(&rules.home);
    if matches!(
        relation(&here, &home),
        Some(Relation::Is | Relation::Contains)
    ) {
        return refuse(
            "Your whole home folder cannot be sent. Choose folders inside it.".to_owned(),
        );
    }
    if let Some((relation, taken)) = super::clash(path, &rules.own) {
        let own = name_of(&taken.place);
        return refuse(match relation {
            Relation::Contains => format!(
                "{} holds OurStick's own folder and cannot be sent.",
                quoted(&name)
            ),
            _ => format!(
                "{} is OurStick's own folder and cannot be sent.",
                quoted(&own)
            ),
        });
    }
    if let Some(private) = private_part(path, &here, &home, rules) {
        return refuse(format!(
            "{} holds private settings and cannot be sent.",
            quoted(&private)
        ));
    }
    if let Some(system) = system_part(&here, rules) {
        return refuse(format!(
            "{} is part of the system and cannot be sent.",
            quoted(&system)
        ));
    }
    for chosen in &rules.chosen {
        let other = name_of(chosen);
        match relation(&here, &comparable(chosen)) {
            Some(Relation::Is) => {
                return refuse(format!("{} is in the list already.", quoted(&name)))
            }
            Some(Relation::IsInside) => {
                return refuse(format!(
                    "{} is inside {}, so it comes along already.",
                    quoted(&name),
                    quoted(&other)
                ))
            }
            Some(Relation::Contains) => {
                return refuse(format!(
                    "{} holds {}, which is in the list already. Take {} out first.",
                    quoted(&name),
                    quoted(&other),
                    quoted(&other)
                ))
            }
            None => {}
        }
    }
    if rules
        .main_folders
        .iter()
        .any(|main| comparable(main) == here)
    {
        return MaySend::Ask(Ask::MainFolder { name });
    }
    if metadata.is_dir() && is_whole_disk(path) {
        return MaySend::Ask(Ask::WholeDisk { name });
    }
    MaySend::Fine
}

fn refuse(why: String) -> MaySend {
    MaySend::Refuse(why)
}

/// The private folder `here` is or lies inside: a top folder of the home
/// whose name starts with a dot, one of `rules.private`, or (Windows) one
/// marked hidden or of the system.
fn private_part(path: &Path, here: &Path, home: &Path, rules: &SendRules) -> Option<String> {
    if let Ok(inside) = here.strip_prefix(home) {
        if let Some(top) = inside.components().next() {
            let top = top.as_os_str().to_string_lossy();
            if top.starts_with('.') {
                return Some(top.into_owned());
            }
        }
    }
    let private = rules.private.iter().find(|private| {
        matches!(
            relation(here, &comparable(private)),
            Some(Relation::Is | Relation::IsInside)
        )
    });
    if let Some(private) = private {
        return Some(name_of(private));
    }
    hidden_part(path)
}

/// The folder `path` is or lies inside that Windows marks hidden or of the
/// system, short of the disk's top (which may be marked so).
#[cfg(windows)]
fn hidden_part(path: &Path) -> Option<String> {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN_OR_SYSTEM: u32 = 0x2 | 0x4;
    path.ancestors()
        .filter(|part| part.parent().is_some())
        .find(|part| {
            std::fs::symlink_metadata(part)
                .is_ok_and(|metadata| metadata.file_attributes() & HIDDEN_OR_SYSTEM != 0)
        })
        .map(name_of)
}

#[cfg(not(windows))]
fn hidden_part(_path: &Path) -> Option<String> {
    None
}

/// The system's folder `here` is, holds or lies inside.
fn system_part(here: &Path, rules: &SendRules) -> Option<String> {
    let not_system = rules.not_system.iter().any(|place| {
        matches!(
            relation(here, &comparable(place)),
            Some(Relation::Is | Relation::IsInside)
        )
    });
    if !not_system {
        if let Some(system) = rules
            .system
            .iter()
            .find(|system| relation(here, &comparable(system)).is_some())
        {
            return Some(name_of(system));
        }
    }
    here.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .find(|part| SYSTEM_NAMES.contains(&part.to_lowercase().as_str()))
        .map(|part| part.into_owned())
}

/// Whether `path` is a disk's top: a drive (`X:\`, a network share), or a
/// folder on another disk than its parent (a mount point, `/Volumes/*`).
pub(crate) fn is_whole_disk(path: &Path) -> bool {
    let Some(parent) = path.parent() else {
        return true;
    };
    match (std::fs::metadata(path), std::fs::metadata(parent)) {
        (Ok(this), Ok(parent)) => match (
            crate::linked::disk_of(&this),
            crate::linked::disk_of(&parent),
        ) {
            (Some(this), Some(parent)) => this != parent,
            _ => false,
        },
        _ => false,
    }
}

/// What to call `path` in a message: its last name, or the whole of it for
/// a disk's top (`C:`, `/`).
fn name_of(path: &Path) -> String {
    match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => {
            let text = path.display().to_string();
            match text.trim_end_matches(['\\', '/']) {
                "" => text,
                trimmed => trimmed.to_owned(),
            }
        }
    }
}

fn quoted(name: &str) -> String {
    format!("\u{201c}{name}\u{201d}")
}

#[cfg(test)]
#[path = "sending_tests.rs"]
mod tests;
