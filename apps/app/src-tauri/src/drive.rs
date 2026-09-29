//! The stick's folder shown as a drive (docs/gui-spec.md, "The stick is a
//! folder"). macOS: a sparse APFS disk image, attached so that Finder shows it
//! with an eject button. Linux: `~/OurStick`, bookmarked in the file
//! manager's sidebar. Eject detaches the image or removes the bookmark;
//! sharing goes on either way.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use ourstick_core::manifest::DRIVE_MARKER;

const NAME: &str = "OurStick";

fn home() -> Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").context("cannot find your home folder")?,
    ))
}

/// Whether the stick's folder is there and is the stick (its marker).
pub fn folder_is_there(root: &Path) -> bool {
    root.join(DRIVE_MARKER).is_file()
}

/// `path` as a `file://` URI, percent-encoded.
#[cfg(any(target_os = "linux", test))]
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

// ---------------------------------------------------------------- Linux

/// GTK bookmarks (Nautilus and other GTK file managers): one `URI name` per
/// line. Adds ours unless it is there.
#[cfg(any(target_os = "linux", test))]
fn with_gtk_bookmark(content: &str, uri: &str) -> String {
    if content
        .lines()
        .any(|line| line.split(' ').next() == Some(uri))
    {
        return content.to_owned();
    }
    let mut content = content.to_owned();
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("{uri} {NAME}\n"));
    content
}

#[cfg(any(target_os = "linux", test))]
fn without_gtk_bookmark(content: &str, uri: &str) -> String {
    content
        .lines()
        .filter(|line| line.split(' ').next() != Some(uri))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// KDE's places (Dolphin): an XBEL file. Ours gets the USB drive icon.
#[cfg(any(target_os = "linux", test))]
fn xbel_bookmark(uri: &str) -> String {
    format!(
        "  <bookmark href=\"{uri}\">\n   <title>{NAME}</title>\n   <info>\n    \
         <metadata owner=\"http://freedesktop.org\">\n     \
         <bookmark:icon name=\"drive-removable-media-usb\"/>\n    </metadata>\n   \
         </info>\n  </bookmark>\n"
    )
}

#[cfg(any(target_os = "linux", test))]
fn with_xbel_bookmark(content: &str, uri: &str) -> Option<String> {
    if content.contains(&format!("<bookmark href=\"{uri}\"")) {
        return Some(content.to_owned());
    }
    let end = content.rfind("</xbel>")?;
    Some(format!(
        "{}{}{}",
        &content[..end],
        xbel_bookmark(uri),
        &content[end..]
    ))
}

#[cfg(any(target_os = "linux", test))]
fn without_xbel_bookmark(content: &str, uri: &str) -> String {
    let start_tag = format!("<bookmark href=\"{uri}\"");
    let Some(start) = content.find(&start_tag) else {
        return content.to_owned();
    };
    let Some(length) = content[start..].find("</bookmark>") else {
        return content.to_owned();
    };
    let end = start + length + "</bookmark>".len();
    // Take the whole lines, so no blank line is left behind.
    let start = content[..start].rfind('\n').map_or(0, |at| at + 1);
    let end = content[end..]
        .find('\n')
        .map_or(content.len(), |at| end + at + 1);
    format!("{}{}", &content[..start], &content[end..])
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use ourstick_core::manifest::ignore::is_ignored;

    fn gtk_bookmarks() -> Result<PathBuf> {
        let config = match std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => home()?.join(".config"),
        };
        Ok(config.join("gtk-3.0/bookmarks"))
    }

    /// Dolphin's places, only if the user has KDE's file (never made here).
    fn kde_places() -> Result<PathBuf> {
        Ok(home()?.join(".local/share/user-places.xbel"))
    }

    fn edit(path: &Path, change: impl Fn(&str) -> Option<String>) -> Result<()> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()))
            }
        };
        if let Some(changed) = change(&content) {
            if changed != content {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, changed)
                    .with_context(|| format!("cannot write {}", path.display()))?;
            }
        }
        Ok(())
    }

    /// Makes `~/OurStick` for a new stick. An existing one is used only if it
    /// is empty (system files aside) or already a stick's; anything else is
    /// the user's and is left alone.
    pub fn create(_size: u64) -> Result<PathBuf> {
        let root = home()?.join(NAME);
        if root.exists() {
            let has_own_files = std::fs::read_dir(&root)
                .with_context(|| format!("cannot open {}", root.display()))?
                .any(|entry| {
                    entry.is_ok_and(|entry| !entry.file_name().to_str().is_some_and(is_ignored))
                });
            if has_own_files && !folder_is_there(&root) {
                bail!(
                    "{} already exists and holds files. OurStick will not take it over; \
                     move it away and try again.",
                    root.display()
                );
            }
        } else {
            std::fs::create_dir(&root)
                .with_context(|| format!("cannot create {}", root.display()))?;
        }
        // A USB drive icon for the folder where the file manager honours it.
        let _ = std::process::Command::new("gio")
            .arg("set")
            .arg(&root)
            .arg("metadata::custom-icon-name")
            .arg("drive-removable-media-usb")
            .status();
        mount(&root)
    }

    /// Puts the stick in the sidebar; the folder itself is always there.
    pub fn mount(root: &Path) -> Result<PathBuf> {
        let uri = file_uri(root);
        edit(&gtk_bookmarks()?, |content| {
            Some(with_gtk_bookmark(content, &uri))
        })?;
        let places = kde_places()?;
        if places.exists() {
            edit(&places, |content| with_xbel_bookmark(content, &uri))?;
        }
        Ok(root.to_path_buf())
    }

    /// Takes the stick out of the sidebar. The folder stays, and so does
    /// sharing.
    pub fn eject(root: &Path) -> Result<()> {
        let uri = file_uri(root);
        edit(&gtk_bookmarks()?, |content| {
            Some(without_gtk_bookmark(content, &uri))
        })?;
        let places = kde_places()?;
        if places.exists() {
            edit(&places, |content| {
                Some(without_xbel_bookmark(content, &uri))
            })?;
        }
        Ok(())
    }

    pub fn is_mounted(root: &Path) -> bool {
        let uri = file_uri(root);
        gtk_bookmarks()
            .and_then(|path| Ok(std::fs::read_to_string(path)?))
            .is_ok_and(|content| {
                content
                    .lines()
                    .any(|line| line.split(' ').next() == Some(uri.as_str()))
            })
    }
}

// ---------------------------------------------------------------- macOS

/// The first `mount-point` in the plist that `hdiutil attach -plist` prints.
#[cfg(any(target_os = "macos", test))]
fn mount_point_in(plist: &str) -> Option<PathBuf> {
    let after_key = plist.split("<key>mount-point</key>").nth(1)?;
    let start = after_key.find("<string>")? + "<string>".len();
    let end = after_key[start..].find("</string>")? + start;
    let path = after_key[start..end]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    Some(PathBuf::from(path))
}

/// `hdiutil` counts `-size …b` in 512-byte sectors.
#[cfg(any(target_os = "macos", test))]
fn sectors(bytes: u64) -> u64 {
    bytes.div_ceil(512)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::process::Command;

    /// Where the disk image lives.
    fn image() -> Result<PathBuf> {
        Ok(home()?
            .join("Library/Application Support/OurStick")
            .join(format!("{NAME}.sparseimage")))
    }

    fn hdiutil(arguments: &[&std::ffi::OsStr]) -> Result<String> {
        let output = Command::new("hdiutil")
            .args(arguments)
            .output()
            .context("cannot run hdiutil")?;
        if !output.status.success() {
            bail!(
                "hdiutil failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Makes the disk image for a new stick of `size` bytes and attaches it.
    pub fn create(size: u64) -> Result<PathBuf> {
        let image = image()?;
        if image.exists() {
            bail!(
                "{} already exists. OurStick will not overwrite it; move it away and \
                 try again.",
                image.display()
            );
        }
        std::fs::create_dir_all(image.parent().expect("the image is in a folder"))?;
        let size = format!("{}b", sectors(size));
        hdiutil(&[
            "create".as_ref(),
            "-type".as_ref(),
            "SPARSE".as_ref(),
            "-fs".as_ref(),
            "APFS".as_ref(),
            "-volname".as_ref(),
            NAME.as_ref(),
            "-size".as_ref(),
            size.as_ref(),
            image.as_os_str(),
        ])?;
        let root = attach(&image)?;
        // Spotlight leaves the stick alone (its files never go into a stick).
        std::fs::write(root.join(".metadata_never_index"), b"")?;
        Ok(root)
    }

    fn attach(image: &Path) -> Result<PathBuf> {
        let plist = hdiutil(&["attach".as_ref(), "-plist".as_ref(), image.as_os_str()])?;
        mount_point_in(&plist).context("hdiutil attached the stick but named no mount point")
    }

    /// Attaches the image; returns where it is mounted (it may differ from
    /// last time, e.g. `/Volumes/OurStick 1`).
    pub fn mount(_root: &Path) -> Result<PathBuf> {
        attach(&image()?)
    }

    pub fn eject(root: &Path) -> Result<()> {
        hdiutil(&["detach".as_ref(), root.as_os_str()])
            .context("cannot eject the stick; close the files that are open on it")?;
        Ok(())
    }

    pub fn is_mounted(root: &Path) -> bool {
        folder_is_there(root)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform {
    use super::*;

    pub fn create(_size: u64) -> Result<PathBuf> {
        bail!("OurStick has no drive for this system yet")
    }
    pub fn mount(root: &Path) -> Result<PathBuf> {
        Ok(root.to_path_buf())
    }
    pub fn eject(_root: &Path) -> Result<()> {
        Ok(())
    }
    pub fn is_mounted(root: &Path) -> bool {
        folder_is_there(root)
    }
}

pub use platform::{create, eject, is_mounted, mount};

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "file:///home/anna/OurStick";

    #[test]
    fn a_path_becomes_a_file_uri() {
        assert_eq!(file_uri(Path::new("/home/anna/OurStick")), URI);
        assert_eq!(
            file_uri(Path::new("/home/a b/Ö")),
            "file:///home/a%20b/%C3%96"
        );
    }

    #[test]
    fn the_gtk_bookmark_is_added_once_and_removed_alone() {
        let theirs = "file:///home/anna/Music\nfile:///home/anna/Work Work\n";
        let with = with_gtk_bookmark(theirs, URI);
        assert_eq!(with, format!("{theirs}{URI} OurStick\n"));
        assert_eq!(with_gtk_bookmark(&with, URI), with);
        assert_eq!(without_gtk_bookmark(&with, URI), theirs);
        assert_eq!(with_gtk_bookmark("", URI), format!("{URI} OurStick\n"));
        // A missing final newline in their file does not glue the lines.
        assert_eq!(
            with_gtk_bookmark("file:///x", URI),
            format!("file:///x\n{URI} OurStick\n")
        );
    }

    #[test]
    fn the_kde_place_is_added_once_and_removed_alone() {
        let theirs = "<?xml version=\"1.0\"?>\n<xbel>\n  <bookmark href=\"file:///home/anna\">\n   <title>Home</title>\n  </bookmark>\n</xbel>\n";
        let with = with_xbel_bookmark(theirs, URI).unwrap();
        assert!(with.contains("drive-removable-media-usb"));
        assert!(with.ends_with("</bookmark>\n</xbel>\n"));
        assert_eq!(with_xbel_bookmark(&with, URI).unwrap(), with);
        assert_eq!(without_xbel_bookmark(&with, URI), theirs);
        assert_eq!(without_xbel_bookmark(theirs, URI), theirs);
        assert_eq!(with_xbel_bookmark("not xbel", URI), None);
    }

    #[test]
    fn the_mount_point_is_read_from_hdiutil() {
        let plist = "<plist><dict><key>system-entities</key><array>\
            <dict><key>dev-entry</key><string>/dev/disk4</string></dict>\
            <dict><key>mount-point</key><string>/Volumes/OurStick 1</string></dict>\
            </array></dict></plist>";
        assert_eq!(
            mount_point_in(plist),
            Some(PathBuf::from("/Volumes/OurStick 1"))
        );
        assert_eq!(mount_point_in("<plist></plist>"), None);
        assert_eq!(sectors(10_000_000_000), 19_531_250);
        assert_eq!(sectors(513), 2);
    }
}
