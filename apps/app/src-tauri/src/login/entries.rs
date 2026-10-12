//! What each system's login entry holds, and how an existing one is read
//! (ADR 0024 §7). Text only, so that every system's is tested everywhere.

use std::path::Path;

use super::{Found, AUTOSTARTED};

#[cfg(any(windows, test))]
/// The name of the Windows value, the same as the one Tauri's NSIS
/// uninstaller removes (`${PRODUCTNAME}`).
pub const WINDOWS_NAME: &str = "OurStick";
#[cfg(any(windows, test))]
pub const RUN: &str = r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(any(windows, test))]
/// Task Manager's own switch for each `Run` value.
pub const APPROVED: &str =
    r"HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
#[cfg(any(target_os = "macos", test))]
/// The macOS LaunchAgent's label, also its file's name.
pub const MACOS_LABEL: &str = "com.ourstick.OurStick";
#[cfg(any(target_os = "linux", test))]
/// The Linux autostart entry's file name.
pub const LINUX_FILE: &str = "ourstick.desktop";

#[cfg(any(windows, test))]
/// The `Run` value: the program in quotes (a path with a space must not be
/// split), then the argument.
pub fn windows_value(exe: &Path) -> String {
    format!("\"{}\" {AUTOSTARTED}", exe.display())
}

#[cfg(any(windows, test))]
/// The `.reg` file that writes the `Run` value; with `clear_off`, Task
/// Manager's switch set to enabled too.
pub fn windows_file(exe: &Path, clear_off: bool) -> String {
    use crate::registry::{file, quoted};
    let mut sections = vec![format!(
        "[{RUN}]\n{}={}",
        quoted(WINDOWS_NAME),
        quoted(&windows_value(exe))
    )];
    if clear_off {
        sections.push(format!(
            "[{APPROVED}]\n{}=hex:02,00,00,00,00,00,00,00,00,00,00,00",
            quoted(WINDOWS_NAME)
        ));
    }
    file(&sections)
}

#[cfg(any(windows, test))]
/// The `.reg` file that removes the `Run` value (Task Manager's switch is
/// left as the user set it).
pub fn windows_removal() -> String {
    crate::registry::file(&[format!(
        "[{RUN}]\n{}=-",
        crate::registry::quoted(WINDOWS_NAME)
    )])
}

#[cfg(any(windows, test))]
/// The entry, from what `reg query` prints for `Run` and for Task
/// Manager's `StartupApproved\Run` (None where there is no such key).
pub fn windows_found(run: Option<&str>, approved: Option<&str>, exe: &Path) -> Option<Found> {
    let ours = |kind: &str, output: Option<&str>| {
        crate::registry::values_in(output.unwrap_or(""), &[kind])
            .into_iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(WINDOWS_NAME))
            .map(|(_, _, data)| data)
    };
    let value = ours("REG_SZ", run)?;
    // Task Manager writes 02… for enabled and 03… for disabled (06/07 too):
    // an odd first byte is off.
    let system_off = ours("REG_BINARY", approved)
        .and_then(|data| u8::from_str_radix(data.get(..2)?, 16).ok())
        .is_some_and(|first| first & 1 == 1);
    Some(Found {
        ours: value == windows_value(exe),
        system_off,
    })
}

#[cfg(any(target_os = "macos", test))]
/// Text in an XML element.
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(any(target_os = "macos", test))]
/// The LaunchAgent: the executable inside the app's bundle, run at login in
/// the user's graphical session, never kept alive (Quit stays quit).
pub fn macos_plist(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{MACOS_LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>{AUTOSTARTED}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>AssociatedBundleIdentifiers</key>
	<array>
		<string>{MACOS_LABEL}</string>
	</array>
</dict>
</plist>
"#,
        xml(&exe.to_string_lossy())
    )
}

#[cfg(any(target_os = "macos", test))]
/// The LaunchAgent as it is; macOS's own switch (Login Items) cannot be
/// read without `unsafe`, so never "off".
pub fn macos_found(plist: Option<&str>, exe: &Path) -> Option<Found> {
    let plist = plist?;
    Some(Found {
        ours: plist == macos_plist(exe),
        system_off: false,
    })
}

#[cfg(any(target_os = "linux", test))]
/// One argument of `Exec=`, by the Desktop Entry rules: in quotes if it
/// holds a reserved character, with `"`, `` ` ``, `$` and `\` escaped.
fn exec_argument(argument: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    if !argument.contains(RESERVED) {
        return argument.to_owned();
    }
    let mut quoted = String::from("\"");
    for character in argument.chars() {
        if matches!(character, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

#[cfg(any(target_os = "linux", test))]
/// The `Exec=` line: the arguments quoted, `%` doubled (field codes),
/// then every `\` doubled again (a string value's escaping).
pub fn linux_exec(exe: &Path) -> String {
    let command =
        format!("{} {AUTOSTARTED}", exec_argument(&exe.to_string_lossy())).replace('%', "%%");
    format!("Exec={}", command.replace('\\', r"\\"))
}

#[cfg(any(target_os = "linux", test))]
/// The autostart entry. No `X-GNOME-Autostart-enabled`: the desktop's own
/// switch writes it (false) when the user turns it off.
pub fn linux_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=OurStick\nComment=Shares your sticks after a restart\nIcon=ourstick-app\n{}\nX-GNOME-Autostart-Delay=3\nTerminal=false\n",
        linux_exec(exe)
    )
}

#[cfg(any(target_os = "linux", test))]
/// The autostart entry as it is: ours if it runs this program; off if the
/// desktop turned it off (GNOME: `X-GNOME-Autostart-enabled=false`; KDE:
/// `Hidden=true`).
pub fn linux_found(entry: Option<&str>, exe: &Path) -> Option<Found> {
    let entry = entry?;
    let exec = linux_exec(exe);
    let has = |line: &str| entry.lines().any(|seen| seen.trim() == line);
    Some(Found {
        ours: has(&exec),
        system_off: has("X-GNOME-Autostart-enabled=false") || has("Hidden=true"),
    })
}
