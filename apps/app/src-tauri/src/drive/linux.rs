//! Linux: each stick a folder in `~/OurStick`, bookmarked under its name in
//! the file manager's sidebar (GTK and KDE). Eject removes the bookmark.

use std::path::Path;

/// `path` as a `file://` URI, percent-encoded.
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

/// A name as GTK's bookmarks file takes it: one line, so no line breaks
/// (a stick's name has none, but a file must never be broken by one).
fn one_line(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).collect()
}

/// `text` safe inside XML.
fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// GTK bookmarks (Nautilus and other GTK file managers): one `URI name` per
/// line. Adds ours, named `name`, unless it is there.
fn with_gtk_bookmark(content: &str, uri: &str, name: &str) -> String {
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
    content.push_str(&format!("{uri} {}\n", one_line(name)));
    content
}

fn without_gtk_bookmark(content: &str, uri: &str) -> String {
    content
        .lines()
        .filter(|line| line.split(' ').next() != Some(uri))
        .map(|line| format!("{line}\n"))
        .collect()
}

/// KDE's places (Dolphin): an XBEL file. Ours gets the USB drive icon.
fn xbel_bookmark(uri: &str, name: &str) -> String {
    let title = xml_escaped(&one_line(name));
    format!(
        "  <bookmark href=\"{uri}\">\n   <title>{title}</title>\n   <info>\n    \
         <metadata owner=\"http://freedesktop.org\">\n     \
         <bookmark:icon name=\"drive-removable-media-usb\"/>\n    </metadata>\n   \
         </info>\n  </bookmark>\n"
    )
}

fn with_xbel_bookmark(content: &str, uri: &str, name: &str) -> Option<String> {
    if content.contains(&format!("<bookmark href=\"{uri}\"")) {
        return Some(content.to_owned());
    }
    let end = content.rfind("</xbel>")?;
    Some(format!(
        "{}{}{}",
        &content[..end],
        xbel_bookmark(uri, name),
        &content[end..]
    ))
}

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
pub mod platform {
    use super::*;
    use crate::drive::{folder_is_there, home, NAME};
    use anyhow::{bail, Context, Result};
    use ourstick_core::OwnedStick;
    use std::path::PathBuf;

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

    /// A USB drive icon for the folder where the file manager honours it.
    /// File managers keep it by path: it is set again whenever the stick is
    /// plugged in or renamed, and taken off `~/OurStick`, which was the
    /// first stick before it moved in.
    fn usb_icon(root: &Path) {
        let _ = std::process::Command::new("gio")
            .arg("set")
            .arg(root)
            .arg("metadata::custom-icon-name")
            .arg("drive-removable-media-usb")
            .output();
        if let Ok(home) = sticks_home() {
            if root.parent() == Some(home.as_path()) {
                let _ = std::process::Command::new("gio")
                    .args(["set", "-t", "unset"])
                    .arg(&home)
                    .arg("metadata::custom-icon-name")
                    .output();
            }
        }
    }

    fn add_bookmark(root: &Path, name: &str) -> Result<()> {
        let uri = file_uri(root);
        edit(&gtk_bookmarks()?, |content| {
            Some(with_gtk_bookmark(content, &uri, name))
        })?;
        let places = kde_places()?;
        if places.exists() {
            edit(&places, |content| with_xbel_bookmark(content, &uri, name))?;
        }
        Ok(())
    }

    fn remove_bookmark(root: &Path) -> Result<()> {
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

    /// Where the app's sticks are: `~/OurStick`, a folder each.
    pub fn sticks_home() -> Result<PathBuf> {
        Ok(home()?.join(NAME))
    }

    /// Makes the folder of a new stick, `~/OurStick/<folder>`, in the
    /// sidebar as `name`. The folder must be new (the caller picks a free
    /// name); the first stick of an older version, `~/OurStick` itself,
    /// must have moved into it first.
    pub fn create(folder: &str, name: &str) -> Result<(PathBuf, Option<PathBuf>)> {
        let home = sticks_home()?;
        if folder_is_there(&home) {
            bail!(
                "the first stick is still {}; it moves first",
                home.display()
            );
        }
        std::fs::create_dir_all(&home)
            .with_context(|| format!("cannot create {}", home.display()))?;
        let root = home.join(folder);
        std::fs::create_dir(&root).with_context(|| format!("cannot create {}", root.display()))?;
        usb_icon(&root);
        add_bookmark(&root, name)?;
        Ok((root, None))
    }

    /// Puts the stick in the sidebar; the folder itself is always there.
    pub fn mount(stick: &OwnedStick) -> Result<PathBuf> {
        usb_icon(&stick.root_path);
        add_bookmark(&stick.root_path, &stick.name)?;
        Ok(stick.root_path.clone())
    }

    /// Takes the stick out of the sidebar. The folder stays, and so does
    /// sharing.
    pub fn eject(stick: &OwnedStick) -> Result<()> {
        remove_bookmark(&stick.root_path)
    }

    /// Renames the stick's folder to `folder` (in the same place) and its
    /// bookmark to `name`. Returns where the folder is now.
    pub fn rename(stick: &OwnedStick, folder: &str, name: &str) -> Result<PathBuf> {
        let root = &stick.root_path;
        let renamed = match root.parent() {
            Some(parent) if root.file_name() != Some(std::ffi::OsStr::new(folder)) => {
                parent.join(folder)
            }
            _ => root.clone(),
        };
        remove_bookmark(root)?;
        if &renamed != root {
            std::fs::rename(root, &renamed).with_context(|| {
                format!("cannot rename {} to {}", root.display(), renamed.display())
            })?;
        }
        usb_icon(&renamed);
        add_bookmark(&renamed, name)?;
        Ok(renamed)
    }

    pub fn is_mounted(stick: &OwnedStick) -> bool {
        let uri = file_uri(&stick.root_path);
        gtk_bookmarks()
            .and_then(|path| Ok(std::fs::read_to_string(path)?))
            .is_ok_and(|content| {
                content
                    .lines()
                    .any(|line| line.split(' ').next() == Some(uri.as_str()))
            })
    }
}

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
        let with = with_gtk_bookmark(theirs, URI, "For Maria");
        assert_eq!(with, format!("{theirs}{URI} For Maria\n"));
        assert_eq!(with_gtk_bookmark(&with, URI, "For Maria"), with);
        assert_eq!(without_gtk_bookmark(&with, URI), theirs);
        assert_eq!(
            with_gtk_bookmark("", URI, "a\nb"),
            format!("{URI} ab\n"),
            "one line, whatever the name"
        );
        // A missing final newline in their file does not glue the lines.
        assert_eq!(
            with_gtk_bookmark("file:///x", URI, "OurStick"),
            format!("file:///x\n{URI} OurStick\n")
        );
    }

    #[test]
    fn the_kde_place_is_added_once_and_removed_alone() {
        let theirs = "<?xml version=\"1.0\"?>\n<xbel>\n  <bookmark href=\"file:///home/anna\">\n   <title>Home</title>\n  </bookmark>\n</xbel>\n";
        let with = with_xbel_bookmark(theirs, URI, "Tom & Jerry <3").unwrap();
        assert!(with.contains("drive-removable-media-usb"));
        assert!(with.contains("<title>Tom &amp; Jerry &lt;3</title>"));
        assert!(with.ends_with("</bookmark>\n</xbel>\n"));
        assert_eq!(with_xbel_bookmark(&with, URI, "x").unwrap(), with);
        assert_eq!(without_xbel_bookmark(&with, URI), theirs);
        assert_eq!(without_xbel_bookmark(theirs, URI), theirs);
        assert_eq!(with_xbel_bookmark("not xbel", URI, "x"), None);
    }
}
