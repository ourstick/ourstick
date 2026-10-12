//! Windows' registry, written the one way the app writes it: a `.reg` file
//! (its escaping fully specified) imported with `reg import`, one process
//! per change; read with `reg query`. Used by File Explorer's entries (ADR
//! 0023) and the start at login (ADR 0024).

/// A `.reg` string: in quotes, with `\` and `"` escaped. No line break may
/// be in it (the callers' texts have none).
pub fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', r"\\").replace('"', "\\\""))
}

/// A `REG_EXPAND_SZ` value: its UTF-16LE bytes and the final NUL, in hex.
pub fn expandable(text: &str) -> String {
    let bytes: Vec<String> = text
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("hex(2):{}", bytes.join(","))
}

/// A `.reg` file of `sections` (`[key]` and its values, lines split by
/// `\n`), with the line ends `reg` expects.
pub fn file(sections: &[String]) -> String {
    let mut text = String::from("Windows Registry Editor Version 5.00\r\n");
    for section in sections {
        text.push_str("\r\n");
        text.push_str(&section.replace('\n', "\r\n"));
        text.push_str("\r\n");
    }
    text
}

/// The string values in what `reg query` prints for a key:
/// `    <name>    <type>    <data>` per value, for the `types` asked;
/// other lines are skipped.
pub fn values_in(output: &str, types: &[&str]) -> Vec<(String, String, String)> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("    "))
        .filter_map(|line| {
            types.iter().find_map(|kind| {
                let (name, data) = line.split_once(&format!("    {kind}    "))?;
                Some((
                    name.to_owned(),
                    (*kind).to_owned(),
                    data.trim_end().to_owned(),
                ))
            })
        })
        .collect()
}

#[cfg(windows)]
pub mod platform {
    use anyhow::{bail, Context, Result};
    use std::path::Path;

    /// Imports `text` as a `.reg` file in `dir` (UTF-16LE with a BOM, as
    /// `reg` reads it whatever the code page), named after `tag`.
    pub fn import(dir: &Path, tag: &str, text: &str) -> Result<()> {
        let path = dir.join(format!("{tag}.reg"));
        let bytes: Vec<u8> = [0xFEFF]
            .into_iter()
            .chain(text.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect();
        std::fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
        let output = crate::drive::no_window("reg")
            .arg("import")
            .arg(&path)
            .arg("/reg:64")
            .output();
        let _ = std::fs::remove_file(&path);
        let output = output.context("cannot run reg")?;
        if !output.status.success() {
            bail!(
                "reg import failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    /// What `reg query` prints for `key`, or None if there is no such key.
    pub fn query(key: &str) -> Option<String> {
        let output = crate::drive::no_window("reg")
            .args(["query", key, "/reg:64"])
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_are_quoted_and_escaped() {
        assert_eq!(quoted(r#"a\b "c""#), r#""a\\b \"c\"""#);
        assert_eq!(quoted("Φ"), "\"Φ\"");
    }

    #[test]
    fn an_expandable_value_is_its_utf16_bytes() {
        assert_eq!(expandable("%A"), "hex(2):25,00,41,00,00,00");
    }

    #[test]
    fn a_file_has_the_header_and_crlf() {
        assert_eq!(
            file(&["[K]\n\"v\"=\"1\"".to_owned()]),
            "Windows Registry Editor Version 5.00\r\n\r\n[K]\r\n\"v\"=\"1\"\r\n"
        );
    }

    #[test]
    fn values_are_read_by_type() {
        let output = "\r\nHKEY_CURRENT_USER\\X\r\n    OurStick    REG_SZ    \"C:\\a b\\o.exe\" --autostarted\r\n    Other    REG_BINARY    030000\r\n    {AB}    REG_SZ    00ff\r\n";
        assert_eq!(
            values_in(output, &["REG_SZ"]),
            [
                (
                    "OurStick".to_owned(),
                    "REG_SZ".to_owned(),
                    "\"C:\\a b\\o.exe\" --autostarted".to_owned()
                ),
                ("{AB}".to_owned(), "REG_SZ".to_owned(), "00ff".to_owned())
            ]
        );
        assert_eq!(values_in(output, &["REG_BINARY"])[0].2, "030000");
        assert!(values_in("", &["REG_SZ"]).is_empty());
    }
}
