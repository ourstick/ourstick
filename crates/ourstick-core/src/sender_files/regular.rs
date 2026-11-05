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

/// Opens `path`, whose last `below` names lie under the stick's root (the
/// root itself, the folder a user chose, may be reached through links).
/// `below` 0: `path` is the root, a linked file.
#[cfg(unix)]
pub(crate) fn open_regular(path: &Path, below: usize) -> io::Result<File> {
    use rustix::fs::{fcntl_getfl, fcntl_setfl, fstat, open, openat, FileType, Mode, OFlags};
    use std::path::Component;

    let errno = |error: rustix::io::Errno| -> io::Error {
        // A link where none may be: a file, or a folder on the way.
        if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
            refused()
        } else {
            error.into()
        }
    };
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
    let file_flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC;
    let file = match names.split_first() {
        // The root is the file itself (a linked file): followed, as chosen.
        None => open(path, file_flags, Mode::empty()).map_err(errno)?,
        Some((last, folders)) => {
            let mut root = path;
            for _ in 0..below {
                root = root.parent().ok_or_else(refused)?;
            }
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

/// [`open_regular`] on Windows: the file must be a regular one, not a
/// link; the folders on the way are not checked one by one.
#[cfg(not(unix))]
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
