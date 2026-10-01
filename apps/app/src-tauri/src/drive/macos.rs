//! macOS: a sparse APFS disk image per stick, attached so that Finder shows
//! it with an eject button, its volume named after the stick. Eject detaches
//! the image. Each image's path is recorded with its stick.

use std::path::PathBuf;

/// The first `mount-point` in the plist that `hdiutil attach -plist` prints.
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

/// The text of the first `<string>` in `xml`, unescaped.
fn first_string(xml: &str) -> Option<String> {
    let start = xml.find("<string>")? + "<string>".len();
    let end = xml[start..].find("</string>")? + start;
    Some(
        xml[start..end]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

/// The whole-disk device (`/dev/diskN`) of `image` in the plist that
/// `hdiutil info -plist` prints; `None` if the image is not attached.
fn device_of(plist: &str, image: &str) -> Option<String> {
    plist
        .split("<key>image-path</key>")
        .skip(1)
        .find(|entry| first_string(entry).as_deref() == Some(image))
        .and_then(|entry| {
            let after_key = entry.split("<key>dev-entry</key>").nth(1)?;
            first_string(after_key)
        })
}

/// The image's size in the text `hdiutil imageinfo` prints
/// (`Total Bytes: 5000000000`).
fn total_bytes_in(info: &str) -> Option<u64> {
    info.lines()
        .find_map(|line| line.trim().strip_prefix("Total Bytes:"))
        .and_then(|bytes| bytes.trim().parse().ok())
}

/// `hdiutil` counts `-size …b` in 512-byte sectors.
fn sectors(bytes: u64) -> u64 {
    bytes.div_ceil(512)
}

#[cfg(target_os = "macos")]
pub mod platform {
    use super::*;
    use crate::drive::{folder_is_there, home, NAME};
    use anyhow::{bail, Context, Result};
    use ourstick_core::OwnedStick;
    use std::path::Path;
    use std::process::Command;

    /// Where the disk images live.
    fn images() -> Result<PathBuf> {
        Ok(home()?.join("Library/Application Support/OurStick"))
    }

    /// The first stick's disk image, made before images were recorded.
    pub fn image_path() -> Result<PathBuf> {
        Ok(images()?.join(format!("{NAME}.sparseimage")))
    }

    /// A stick's disk image, as recorded (with its row, or at start for
    /// the first stick of an older version); never guessed, so no stick
    /// can take another's.
    fn image_of(stick: &OwnedStick) -> Result<PathBuf> {
        stick
            .image_path
            .clone()
            .context("the stick's disk image is not recorded; restart OurStick")
    }

    /// Where volumes appear: a new stick's volume name must not be taken
    /// there.
    pub fn sticks_home() -> Result<PathBuf> {
        Ok(PathBuf::from("/Volumes"))
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

    /// The folder that holds the stick's disk image: what is left for the
    /// stick is measured there.
    pub fn measure_at(stick: &OwnedStick) -> Result<PathBuf> {
        Ok(image_of(stick)?
            .parent()
            .expect("the image is in a folder")
            .to_path_buf())
    }

    /// The size of the disk that holds the image: a sparse image takes room
    /// only as it fills, so it never limits first (ADR 0008).
    fn disk_size(image: &Path) -> Result<u64> {
        use ourstick_core::space::{Disks, SystemDisks};
        SystemDisks
            .total(image)
            .with_context(|| format!("cannot measure the disk of {}", image.display()))
    }

    /// Makes the disk image of a new stick, as large as its disk (sparse),
    /// its volume called `folder`, under a name of its own (recorded with
    /// the stick, never derived again), and attaches it.
    pub fn create(folder: &str, _name: &str) -> Result<(PathBuf, Option<PathBuf>)> {
        let suffix: String = ourstick_core::device::random_key()[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let image = images()?.join(format!("stick-{suffix}.sparseimage"));
        if image.exists() {
            bail!("{} already exists; try again", image.display());
        }
        std::fs::create_dir_all(images()?)?;
        let size = format!("{}b", sectors(disk_size(&image)?));
        hdiutil(&[
            "create".as_ref(),
            "-type".as_ref(),
            "SPARSE".as_ref(),
            "-fs".as_ref(),
            "APFS".as_ref(),
            "-volname".as_ref(),
            folder.as_ref(),
            "-size".as_ref(),
            size.as_ref(),
            image.as_os_str(),
        ])?;
        let root = attach(&image)?;
        // Spotlight leaves the stick alone (its files never go into a stick).
        std::fs::write(root.join(".metadata_never_index"), b"")?;
        Ok((root, Some(image)))
    }

    fn attach(image: &Path) -> Result<PathBuf> {
        let plist = hdiutil(&["attach".as_ref(), "-plist".as_ref(), image.as_os_str()])?;
        mount_point_in(&plist).context("hdiutil attached the stick but named no mount point")
    }

    /// Attaches the stick's image; returns where it is mounted (it may
    /// differ from last time, e.g. `/Volumes/OurStick 1`). An image made
    /// with a size (before ADR 0008) is first grown to its disk's size; if
    /// that fails it stays as it is (growing never touches what is on it).
    pub fn mount(stick: &OwnedStick) -> Result<PathBuf> {
        let image = image_of(stick)?;
        if let Err(error) = grow(&image) {
            tracing::warn!(%error, "the stick's disk image could not be grown");
        }
        attach(&image)
    }

    /// Grows the image to its disk's size, if it is smaller: an image made
    /// with a size before ADR 0008 stays attached after Quit, so it is
    /// detached first (`hdiutil` resizes only a detached image). If it
    /// cannot be detached (a file on it is open), it stays as it is.
    fn grow(image: &Path) -> Result<()> {
        let want = sectors(disk_size(image)?);
        let info = hdiutil(&["imageinfo".as_ref(), image.as_os_str()])?;
        let has = total_bytes_in(&info).context("hdiutil told no image size")?;
        if has >= want * 512 {
            return Ok(());
        }
        let resolved = std::fs::canonicalize(image).unwrap_or_else(|_| image.to_path_buf());
        let attached = hdiutil(&["info".as_ref(), "-plist".as_ref()])?;
        if let Some(device) = device_of(&attached, &resolved.to_string_lossy()) {
            hdiutil(&["detach".as_ref(), device.as_ref()])
                .context("the stick is in use, so it keeps its old size for now")?;
        }
        let size = format!("{want}b");
        hdiutil(&[
            "resize".as_ref(),
            "-size".as_ref(),
            size.as_ref(),
            image.as_os_str(),
        ])?;
        Ok(())
    }

    /// Detaches the stick's image by its device, not by the folder: the
    /// folder's path can change between mounts (`/Volumes/OurStick 1`).
    pub fn eject(stick: &OwnedStick) -> Result<()> {
        let image = image_of(stick)?;
        let image = std::fs::canonicalize(&image).unwrap_or(image);
        let info = hdiutil(&["info".as_ref(), "-plist".as_ref()])?;
        let Some(device) = device_of(&info, &image.to_string_lossy()) else {
            // Not attached: already ejected.
            return Ok(());
        };
        hdiutil(&["detach".as_ref(), device.as_ref()])
            .context("cannot eject the stick; if a file on it is open, close it and try again")?;
        Ok(())
    }

    /// Renames the stick's volume to `folder` (`diskutil rename`). Its
    /// mount point keeps the old name until it is attached again; the image
    /// file keeps its own. A detached image is renamed at its next attach.
    /// Returns where the volume is now.
    pub fn rename(stick: &OwnedStick, folder: &str, _name: &str) -> Result<PathBuf> {
        if !folder_is_there(&stick.root_path) {
            bail!("the stick is ejected; mount it, then rename it");
        }
        let output = Command::new("diskutil")
            .arg("rename")
            .arg(&stick.root_path)
            .arg(folder)
            .output()
            .context("cannot run diskutil")?;
        if !output.status.success() {
            bail!(
                "diskutil could not rename the stick: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(stick.root_path.clone())
    }

    pub fn is_mounted(stick: &OwnedStick) -> bool {
        folder_is_there(&stick.root_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_size_is_read_from_imageinfo() {
        let info =
            "Format: UDSP\nSize Information:\n\tTotal Bytes: 5000000000\n\tSector Count: 9765625\n";
        assert_eq!(total_bytes_in(info), Some(5_000_000_000));
        assert_eq!(total_bytes_in("Format: UDSP\n"), None);
    }

    #[test]
    fn the_device_is_found_by_its_image() {
        let plist = "<plist><dict><key>images</key><array>\
            <dict><key>image-path</key><string>/Users/a/Other.dmg</string>\
            <key>system-entities</key><array>\
            <dict><key>dev-entry</key><string>/dev/disk3</string></dict></array></dict>\
            <dict><key>image-path</key><string>/Users/a/Library/Application Support/OurStick/OurStick.sparseimage</string>\
            <key>system-entities</key><array>\
            <dict><key>dev-entry</key><string>/dev/disk4</string></dict>\
            <dict><key>dev-entry</key><string>/dev/disk4s1</string></dict></array></dict>\
            </array></dict></plist>";
        let ours = "/Users/a/Library/Application Support/OurStick/OurStick.sparseimage";
        assert_eq!(device_of(plist, ours).as_deref(), Some("/dev/disk4"));
        assert_eq!(
            device_of(plist, "/Users/a/Other.dmg").as_deref(),
            Some("/dev/disk3")
        );
        assert_eq!(device_of(plist, "/Users/a/Missing.dmg"), None);
        assert_eq!(device_of("<plist></plist>", ours), None);
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
