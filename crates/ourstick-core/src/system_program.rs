//! Windows' own programs, by their whole path.

use std::path::PathBuf;

/// Where Windows keeps its program `name` (`subst`, `reg`, `attrib`):
/// `%SystemRoot%\System32\<name>.exe`. Run by this path, never looked for
/// on the PATH, where a folder of the user's could hold another program of
/// that name.
pub fn system_program(name: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("System32").join(format!("{name}.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_system_program_is_in_system32() {
        let attrib = system_program("attrib");
        assert_eq!(attrib.file_name().unwrap(), "attrib.exe");
        assert_eq!(attrib.parent().unwrap().file_name().unwrap(), "System32");
        assert!(attrib.parent().unwrap().parent().is_some());
    }
}
