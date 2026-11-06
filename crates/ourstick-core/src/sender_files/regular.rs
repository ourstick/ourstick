//! Opening a shared file as the walk saw it: a regular file, reached
//! without following a link anywhere below its root, and never blocking on
//! a named pipe. Something swapped in after the walk (a link to a file
//! outside the stick, a link in place of one of its folders, a pipe) is
//! refused, not read.

use std::fs::File;
use std::io;
use std::path::Path;

fn refused() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not a regular file")
}

/// The last `below` names of `path`, the file's last, and the root above
/// them; refused if any is not a plain name.
fn split_below(path: &Path, below: usize) -> io::Result<(&Path, Vec<&std::ffi::OsStr>)> {
    use std::path::Component;

    let names: Vec<&std::ffi::OsStr> = path
        .components()
        .rev()
        .take(below)
        .map(|component| match component {
            Component::Normal(name) => Ok(name),
            _ => Err(refused()),
        })
        .collect::<io::Result<_>>()?;
    if names.len() != below {
        return Err(refused());
    }
    let mut root = path;
    for _ in 0..below {
        root = root.parent().ok_or_else(refused)?;
    }
    Ok((root, names))
}

/// Opens `path`, whose last `below` names lie under the stick's root (the
/// root itself, the folder a user chose, may be reached through links).
/// `below` 0: `path` is the root, a linked file.
#[cfg(unix)]
pub(crate) fn open_regular(path: &Path, below: usize) -> io::Result<File> {
    use rustix::fs::{fcntl_getfl, fcntl_setfl, fstat, open, openat, FileType, Mode, OFlags};

    let errno = |error: rustix::io::Errno| -> io::Error {
        // A link where none may be: a file, or a folder on the way.
        if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
            refused()
        } else {
            error.into()
        }
    };
    let (root, names) = split_below(path, below)?;
    let file_flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = match names.split_first() {
        // The root is the file itself (a linked file): followed, as chosen.
        None => open(path, file_flags, Mode::empty()).map_err(errno)?,
        Some((last, folders)) => {
            let folder_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
            let mut folder = open(root, folder_flags, Mode::empty()).map_err(errno)?;
            for name in folders.iter().rev() {
                folder = openat(
                    &folder,
                    *name,
                    folder_flags | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(errno)?;
            }
            openat(&folder, *last, file_flags | OFlags::NOFOLLOW, Mode::empty()).map_err(errno)?
        }
    };
    if FileType::from_raw_mode(fstat(&file)?.st_mode) != FileType::RegularFile {
        return Err(refused());
    }
    // Reads block as usual from here: only the open had to not wait.
    fcntl_setfl(&file, fcntl_getfl(&file)? - OFlags::NONBLOCK)?;
    Ok(File::from(file))
}

/// [`open_regular`] on Windows. Each folder from the root down is opened
/// as itself, not followed: a link or a junction (a name surrogate, what
/// std's `is_symlink` means) is refused, while a folder that is a reparse
/// point of another kind (OneDrive's) is gone through as any folder. The
/// folders are held open without sharing delete until the file is open:
/// none of them can be renamed or swapped in the meantime, so the file's
/// path goes through the very folders that were checked. The file is opened
/// as itself too and must be a regular one, not a link. A file that is a
/// reparse point of another kind (a OneDrive file, a deduplicated one) gives
/// its content only through its own driver, which an open as itself skips:
/// it is opened again the usual way while its name is held. Through std's
/// open options: no unsafe code.
#[cfg(windows)]
pub(crate) fn open_regular(path: &Path, below: usize) -> io::Result<File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    // winbase.h / winnt.h.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const FILE_SHARE_DELETE: u32 = 0x4;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    // `as_itself`: a link or a junction is opened, not followed, and then
    // refused. `held`: without sharing delete, so that while it is open its
    // name cannot be renamed, removed or replaced. Backup semantics on a
    // file too: a folder in a file's place then opens, and is refused by
    // the caller as not a file.
    let open = |path: &Path, as_itself: bool, held: bool| {
        let mut flags = FILE_FLAG_BACKUP_SEMANTICS;
        if as_itself {
            flags |= FILE_FLAG_OPEN_REPARSE_POINT;
        }
        let mut share = FILE_SHARE_READ | FILE_SHARE_WRITE;
        if !held {
            share |= FILE_SHARE_DELETE;
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(flags)
            .share_mode(share)
            .open(path)?;
        let facts = file.metadata()?;
        if facts.file_type().is_symlink() {
            return Err(refused());
        }
        Ok::<_, io::Error>((file, facts))
    };
    let folder = |path: &Path, as_itself: bool| {
        let (file, facts) = open(path, as_itself, true)?;
        facts.is_dir().then_some(file).ok_or_else(refused)
    };
    let regular = |(file, facts): (File, std::fs::Metadata)| {
        facts.is_file().then_some(file).ok_or_else(refused)
    };

    let (root, names) = split_below(path, below)?;
    if names.is_empty() {
        // The root is the file itself (a linked file): followed, as chosen.
        return regular(open(path, false, false)?);
    }
    // The root (the folder the user chose) may be reached through links.
    let mut held = vec![folder(root, false)?];
    let mut on_the_way = root.to_path_buf();
    for name in names[1..].iter().rev() {
        on_the_way.push(name);
        held.push(folder(&on_the_way, true)?);
    }
    let (itself, facts) = open(path, true, false)?;
    if !facts.is_file() {
        return Err(refused());
    }
    let file = if facts.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        itself
    } else {
        drop(itself);
        // Its name held as itself (checked again), then opened the usual
        // way, which can only reach that same file.
        let name = regular(open(path, true, true)?)?;
        let file = regular(open(path, false, false)?)?;
        drop(name);
        file
    };
    drop(held);
    Ok(file)
}

/// [`open_regular`] elsewhere: the file must be a regular one, not a link.
#[cfg(not(any(unix, windows)))]
pub(crate) fn open_regular(path: &Path, _below: usize) -> io::Result<File> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        return Err(refused());
    }
    File::open(path)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("root/sub")).unwrap();
        std::fs::write(dir.path().join("root/sub/a.txt"), b"in the stick").unwrap();
        std::fs::create_dir(dir.path().join("outside")).unwrap();
        std::fs::write(dir.path().join("outside/a.txt"), b"not in the stick").unwrap();
        dir
    }

    #[test]
    fn a_file_under_its_root_is_read() {
        let dir = folder();
        let path = dir.path().join("root/sub/a.txt");
        let file = open_regular(&path, 2).unwrap();
        assert_eq!(std::io::read_to_string(file).unwrap(), "in the stick");
        // The root itself may be reached through a link (the user chose it).
        std::os::unix::fs::symlink(dir.path().join("root"), dir.path().join("chosen")).unwrap();
        assert!(open_regular(&dir.path().join("chosen/sub/a.txt"), 2).is_ok());
        // A linked file is its own root.
        assert!(open_regular(&path, 0).is_ok());
    }

    #[test]
    fn a_link_in_place_of_a_folder_or_the_file_is_refused() {
        let dir = folder();
        // The stick's folder swapped for a link to one outside it.
        std::fs::rename(dir.path().join("root/sub"), dir.path().join("old")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside"), dir.path().join("root/sub"))
            .unwrap();
        let error = open_regular(&dir.path().join("root/sub/a.txt"), 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // The file swapped for a link.
        std::fs::remove_file(dir.path().join("root/sub")).unwrap();
        std::fs::create_dir(dir.path().join("root/sub")).unwrap();
        std::os::unix::fs::symlink(
            dir.path().join("outside/a.txt"),
            dir.path().join("root/sub/a.txt"),
        )
        .unwrap();
        let error = open_regular(&dir.path().join("root/sub/a.txt"), 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_pipe_is_refused_without_waiting() {
        let dir = folder();
        let pipe = dir.path().join("root/sub/pipe");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status();
        if !made.is_ok_and(|status| status.success()) {
            return;
        }
        let error = open_regular(&pipe, 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    /// A junction (no rights needed to make one, unlike a link).
    fn junction(at: &Path, to: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at)
            .arg(to)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("root/sub")).unwrap();
        std::fs::write(dir.path().join("root/sub/a.txt"), b"in the stick").unwrap();
        std::fs::create_dir(dir.path().join("outside")).unwrap();
        std::fs::write(dir.path().join("outside/a.txt"), b"not in the stick").unwrap();
        dir
    }

    #[test]
    fn a_file_under_its_root_is_read_and_the_root_may_be_a_junction() {
        let dir = folder();
        let path = dir.path().join("root/sub/a.txt");
        let file = open_regular(&path, 2).unwrap();
        assert_eq!(std::io::read_to_string(file).unwrap(), "in the stick");
        junction(&dir.path().join("chosen"), &dir.path().join("root"));
        assert!(open_regular(&dir.path().join("chosen/sub/a.txt"), 2).is_ok());
        assert!(open_regular(&path, 0).is_ok());
    }

    #[test]
    fn a_junction_in_place_of_a_folder_is_refused() {
        let dir = folder();
        let sub = dir.path().join("root").join("sub");
        std::fs::rename(&sub, dir.path().join("old")).unwrap();
        junction(&sub, &dir.path().join("outside"));
        let error = open_regular(&dir.path().join("root/sub/a.txt"), 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_folder_on_the_way_cannot_be_swapped_while_it_is_held() {
        let dir = folder();
        use std::os::windows::fs::OpenOptionsExt;
        // Held as open_regular holds it, between the check and the open.
        let held = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .share_mode(0x1 | 0x2)
            .open(dir.path().join("root/sub"))
            .unwrap();
        let moved = std::fs::rename(dir.path().join("root/sub"), dir.path().join("old"));
        assert!(moved.is_err(), "renamed while held");
        drop(held);
        assert!(std::fs::rename(dir.path().join("root/sub"), dir.path().join("old")).is_ok());
    }

    #[test]
    fn a_folder_is_not_a_file() {
        let dir = folder();
        let error = open_regular(&dir.path().join("root/sub"), 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_link_in_place_of_the_file_is_refused() {
        let dir = folder();
        let file = dir.path().join("root").join("sub").join("a.txt");
        std::fs::remove_file(&file).unwrap();
        // A link to a file needs Developer Mode or an administrator.
        let outside = dir.path().join("outside").join("a.txt");
        if std::os::windows::fs::symlink_file(&outside, &file).is_err() {
            return;
        }
        let error = open_regular(&file, 2).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
