use super::entries::*;
use super::*;
use std::path::PathBuf;

fn program(system: System, exe: &Path) -> Program<'_> {
    Program {
        system,
        exe,
        home: Some(Path::new("/Users/alice")),
        debug: false,
        own_data_dir: false,
        appimage: false,
    }
}

#[test]
fn only_the_login_entrys_start_is_autostarted() {
    assert!(autostarted(&["ourstick-app", AUTOSTARTED]));
    assert!(!autostarted(&["ourstick-app"]));
    // The program's own name is not an argument.
    assert!(!autostarted(&[AUTOSTARTED]));
    assert!(!autostarted::<&str>(&[]));
}

#[test]
fn only_the_installed_app_writes_the_entry() {
    let windows =
        PathBuf::from(r"C:\Users\Maria Papadopoulou\AppData\Local\OurStick\ourstick-app.exe");
    let mac = PathBuf::from("/Applications/OurStick.app/Contents/MacOS/ourstick-app");
    let linux = PathBuf::from("/usr/bin/ourstick-app");
    for (system, exe) in [
        (System::Windows, &windows),
        (System::MacOs, &mac),
        (System::Linux, &linux),
    ] {
        assert!(may_write(&program(system, exe)), "{exe:?}");
        for not in [
            Program {
                debug: true,
                ..program(system, exe)
            },
            Program {
                own_data_dir: true,
                ..program(system, exe)
            },
            Program {
                appimage: true,
                ..program(system, exe)
            },
        ] {
            assert!(!may_write(&not), "{not:?}");
        }
    }
    let user_mac =
        PathBuf::from("/Users/alice/Applications/OurStick.app/Contents/MacOS/ourstick-app");
    assert!(may_write(&program(System::MacOs, &user_mac)));
    for exe in [
        r"C:\src\ourstick\target\release\ourstick-app.exe",
        r"D:\cargo-target\TARGET\release\ourstick-app.exe",
    ] {
        assert!(
            !may_write(&program(System::Windows, Path::new(exe))),
            "{exe}"
        );
    }
    for exe in [
        "/Volumes/OurStick/OurStick.app/Contents/MacOS/ourstick-app",
        "/private/var/folders/x/T/AppTranslocation/1234/d/OurStick.app/Contents/MacOS/ourstick-app",
        "/Applications/ourstick-app",
        "/Users/alice/Downloads/OurStick.app/Contents/MacOS/ourstick-app",
    ] {
        assert!(!may_write(&program(System::MacOs, Path::new(exe))), "{exe}");
    }
    for exe in [
        "/home/alice/cargo-target/release/ourstick-app",
        "/usr/local/bin/ourstick-app",
    ] {
        assert!(!may_write(&program(System::Linux, Path::new(exe))), "{exe}");
    }
}

#[test]
fn a_start_writes_only_what_differs_and_keeps_the_systems_off() {
    let ours = Some(Found {
        ours: true,
        system_off: false,
    });
    let elsewhere = Some(Found {
        ours: false,
        system_off: false,
    });
    let off_by_system = Some(Found {
        ours: false,
        system_off: true,
    });
    assert_eq!(at_start(true, None), Step::Write { clear_off: false });
    assert_eq!(at_start(true, ours), Step::Leave);
    assert_eq!(at_start(true, elsewhere), Step::Write { clear_off: false });
    assert_eq!(at_start(true, off_by_system), Step::Leave);
    assert_eq!(at_start(false, None), Step::Leave);
    assert_eq!(at_start(false, ours), Step::Remove);
    assert_eq!(at_start(false, off_by_system), Step::Remove);
    // The user's own act in OurStick undoes the system's off.
    assert_eq!(by_user(true), Step::Write { clear_off: true });
    assert_eq!(by_user(false), Step::Remove);
    assert!(shown_on(true, None));
    assert!(shown_on(true, ours));
    assert!(!shown_on(true, off_by_system));
    assert!(!shown_on(false, ours));
}

#[test]
fn the_windows_value_quotes_the_path() {
    let exe = Path::new(r"C:\Users\Μαρία Π\AppData\Local\OurStick\ourstick-app.exe");
    assert_eq!(
        windows_value(exe),
        r#""C:\Users\Μαρία Π\AppData\Local\OurStick\ourstick-app.exe" --autostarted"#
    );
    assert_eq!(
        windows_file(exe, false),
        format!(
            "Windows Registry Editor Version 5.00\r\n\r\n[{RUN}]\r\n\"OurStick\"=\"\\\"C:\\\\Users\\\\Μαρία Π\\\\AppData\\\\Local\\\\OurStick\\\\ourstick-app.exe\\\" --autostarted\"\r\n"
        )
    );
    let cleared = windows_file(exe, true);
    assert!(cleared.ends_with(&format!(
        "\r\n[{APPROVED}]\r\n\"OurStick\"=hex:02,00,00,00,00,00,00,00,00,00,00,00\r\n"
    )));
    assert_eq!(
        windows_removal(),
        format!("Windows Registry Editor Version 5.00\r\n\r\n[{RUN}]\r\n\"OurStick\"=-\r\n")
    );
}

#[test]
fn the_windows_entry_and_task_managers_switch_are_read() {
    let exe = Path::new(r"C:\a b\ourstick-app.exe");
    let run = format!(
        "\r\n{RUN}\r\n    OneDrive    REG_SZ    x\r\n    OurStick    REG_SZ    \"C:\\a b\\ourstick-app.exe\" --autostarted\r\n"
    );
    let off =
        format!("\r\n{APPROVED}\r\n    OurStick    REG_BINARY    030000000000000000000000\r\n");
    let on =
        format!("\r\n{APPROVED}\r\n    OurStick    REG_BINARY    020000000000000000000000\r\n");
    assert_eq!(
        windows_found(Some(&run), None, exe),
        Some(Found {
            ours: true,
            system_off: false
        })
    );
    assert!(
        windows_found(Some(&run), Some(&off), exe)
            .unwrap()
            .system_off
    );
    assert!(
        !windows_found(Some(&run), Some(&on), exe)
            .unwrap()
            .system_off
    );
    assert!(
        !windows_found(Some(&run), None, Path::new(r"C:\elsewhere.exe"))
            .unwrap()
            .ours
    );
    assert_eq!(windows_found(None, Some(&off), exe), None);
    assert_eq!(windows_found(Some("\r\nRUN\r\n"), None, exe), None);
}

#[test]
fn the_launch_agent_escapes_its_path_and_never_keeps_alive() {
    let exe = Path::new("/Applications/A & <B>.app/Contents/MacOS/ourstick-app");
    let plist = macos_plist(exe);
    assert!(plist.contains("<key>Label</key>\n\t<string>com.ourstick.OurStick</string>"));
    assert!(plist.contains(
        "<string>/Applications/A &amp; &lt;B&gt;.app/Contents/MacOS/ourstick-app</string>\n\t\t<string>--autostarted</string>"
    ));
    assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
    assert!(plist.contains("<string>Aqua</string>"));
    assert!(!plist.contains("KeepAlive"));
    assert_eq!(
        macos_found(Some(&plist), exe),
        Some(Found {
            ours: true,
            system_off: false
        })
    );
    assert!(
        !macos_found(
            Some(&plist),
            Path::new("/Applications/X.app/Contents/MacOS/x")
        )
        .unwrap()
        .ours
    );
    assert_eq!(macos_found(None, exe), None);
}

#[test]
fn the_linux_exec_line_is_escaped_in_both_layers() {
    assert_eq!(
        linux_exec(Path::new("/usr/bin/ourstick-app")),
        "Exec=/usr/bin/ourstick-app --autostarted"
    );
    // Quoted for a space; `$` and `\` escaped by `\`, then every `\`
    // doubled; `%` doubled.
    assert_eq!(
        linux_exec(Path::new("/opt/My Apps/a$b\\c\"d`e 100%/ourstick-app")),
        r#"Exec="/opt/My Apps/a\\$b\\\\c\\"d\\`e 100%%/ourstick-app" --autostarted"#
    );
    // The same name as the app's own menu entry, ourstick.desktop.
    assert_eq!(LINUX_FILE, "ourstick.desktop");
    let entry = linux_entry(Path::new("/usr/bin/ourstick-app"));
    assert!(entry.starts_with("[Desktop Entry]\nType=Application\nName=OurStick\n"));
    assert!(entry.contains("\nIcon=ourstick-app\n"));
    assert!(!entry.contains("X-GNOME-Autostart-enabled"));
}

#[test]
fn the_linux_entry_and_the_desktops_switch_are_read() {
    let exe = Path::new("/usr/bin/ourstick-app");
    let entry = linux_entry(exe);
    assert_eq!(
        linux_found(Some(&entry), exe),
        Some(Found {
            ours: true,
            system_off: false
        })
    );
    let gnome_off = format!("{entry}X-GNOME-Autostart-enabled=false\n");
    let kde_off = format!("{entry}Hidden=true\n");
    assert!(linux_found(Some(&gnome_off), exe).unwrap().system_off);
    assert!(linux_found(Some(&kde_off), exe).unwrap().system_off);
    assert!(
        !linux_found(Some(&entry), Path::new("/usr/local/bin/x"))
            .unwrap()
            .ours
    );
    assert_eq!(linux_found(None, exe), None);
}
