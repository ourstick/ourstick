use super::*;
use std::path::PathBuf;

fn stick(seed: u8) -> StickId {
    ourstick_core::StickSigningKey::from_bytes(&[seed; 32]).stick_id()
}

#[test]
fn a_sticks_class_id_is_stable_braced_and_its_own() {
    let id = class_id(&stick(1));
    assert_eq!(id, class_id(&stick(1)));
    assert_ne!(id, class_id(&stick(2)));
    assert_eq!(id.len(), 38);
    let groups: Vec<usize> = id
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split('-')
        .map(str::len)
        .collect();
    assert_eq!(groups, [8, 4, 4, 4, 12]);
    assert!(id
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase() || "{}-".contains(c)));
}

#[test]
fn each_data_folder_has_its_own_list() {
    let installed = list_name(Path::new(r"C:\Users\anna\AppData\Local\OurStick"));
    assert_eq!(
        installed,
        list_name(Path::new(r"C:\Users\anna\AppData\Local\OurStick"))
    );
    assert_ne!(installed, list_name(Path::new(r"C:\dev\data")));
    assert_eq!(installed.len(), 16);
}

fn entry_for(name: &str, folder: &str) -> String {
    let id = stick(1);
    let folder = PathBuf::from(folder);
    let icon = PathBuf::from(r"C:\Program Files\OurStick\ourstick-app.exe");
    entry_file(
        &Entry {
            stick_id: &id,
            name,
            folder: &folder,
            icon: &icon,
        },
        "0123456789abcdef",
    )
}

#[test]
fn the_entry_file_is_microsofts_recipe_with_the_list_first() {
    let id = class_id(&stick(1));
    let hex = crate::state::stick_hex(&stick(1));
    let text = entry_for("Holiday photos", r"C:\Users\anna\OurStick\Holiday photos");
    let class = |classes: &str| {
        format!(
            "\r\n[{classes}\\{id}]\r\n@=\"Holiday photos\"\r\n\
             \"System.IsPinnedToNameSpaceTree\"=dword:00000001\r\n\
             \"SortOrderIndex\"=dword:00000042\r\n\
             \r\n[{classes}\\{id}\\DefaultIcon]\r\n\
             @=\"C:\\\\Program Files\\\\OurStick\\\\ourstick-app.exe,0\"\r\n\
             \r\n[{classes}\\{id}\\InProcServer32]\r\n@={}\r\n\
             \r\n[{classes}\\{id}\\Instance]\r\n\
             \"CLSID\"=\"{{0E5AAE11-A475-4c5b-AB00-C66DE400274E}}\"\r\n\
             \r\n[{classes}\\{id}\\Instance\\InitPropertyBag]\r\n\
             \"Attributes\"=dword:00000011\r\n\
             \"TargetFolderPath\"=\"C:\\\\Users\\\\anna\\\\OurStick\\\\Holiday photos\"\r\n\
             \r\n[{classes}\\{id}\\ShellFolder]\r\n\
             \"FolderValueFlags\"=dword:00000028\r\n\"Attributes\"=dword:f080004d\r\n",
            expandable(SHELL32)
        )
    };
    let expected = format!(
        "Windows Registry Editor Version 5.00\r\n\
         \r\n[{LISTS}\\0123456789abcdef]\r\n\"{id}\"=\"{hex}\"\r\n\
         {}{}\
         \r\n[{EXPLORER}\\Desktop\\NameSpace\\{id}]\r\n@=\"Holiday photos\"\r\n\
         \r\n[{EXPLORER}\\HideDesktopIcons\\NewStartPanel]\r\n\"{id}\"=dword:00000001\r\n",
        class(r"HKEY_CURRENT_USER\Software\Classes\CLSID"),
        class(r"HKEY_CURRENT_USER\Software\Classes\WOW6432Node\CLSID"),
    );
    assert_eq!(text, expected);
}

#[test]
fn only_shell32s_path_is_expanded() {
    assert_eq!(expandable("%A"), "hex(2):25,00,41,00,00,00");
    // Every other value is a plain string: a `%` in a name or a folder
    // stays as it is.
    let text = entry_for("100% %PATH%", r"C:\Users\anna\OurStick\100% %PATH%");
    // Once per registry view.
    assert_eq!(text.matches("hex(2):").count(), 2);
    assert!(text.contains("@=\"100% %PATH%\"\r\n"));
    assert!(text
        .contains("\"TargetFolderPath\"=\"C:\\\\Users\\\\anna\\\\OurStick\\\\100% %PATH%\"\r\n"));
}

#[test]
fn any_name_arrives_whole() {
    for (name, written) in [
        (r#"Say "hi""#, r#""Say \"hi\"""#),
        (r"a\b\", r#""a\\b\\""#),
        ("/v x", r#""/v x""#),
        ("-d", r#""-d""#),
        ("Φωτογραφίες διακοπών", r#""Φωτογραφίες διακοπών""#),
    ] {
        assert_eq!(quoted(name), written, "{name}");
    }
    let text = entry_for(r#"Say "hi""#, r"C:\x");
    assert_eq!(text.matches(r#"@="Say \"hi\"""#).count(), 3);
}

#[test]
fn the_removal_file_removes_the_entry_then_its_list_value() {
    let id = class_id(&stick(1));
    let text = removal_file(&id, "0123456789abcdef");
    let expected = format!(
        "Windows Registry Editor Version 5.00\r\n\
         \r\n[-{CLASSES}\\{id}]\r\n\
         \r\n[-{CLASSES_32}\\{id}]\r\n\
         \r\n[-{EXPLORER}\\Desktop\\NameSpace\\{id}]\r\n\
         \r\n[{EXPLORER}\\HideDesktopIcons\\NewStartPanel]\r\n\"{id}\"=-\r\n\
         \r\n[{LISTS}\\0123456789abcdef]\r\n\"{id}\"=-\r\n"
    );
    assert_eq!(text, expected);
}

#[test]
fn odd_list_values_go_alone() {
    let text = odd_values_file(&[r#"x"y"#.to_owned()], "0123456789abcdef");
    assert_eq!(
        text,
        format!(
            "Windows Registry Editor Version 5.00\r\n\
             \r\n[{LISTS}\\0123456789abcdef]\r\n\"x\\\"y\"=-\r\n"
        )
    );
    assert!(!text.contains("[-"));
}

#[test]
fn the_list_is_read_from_reg_query() {
    let output = "\r\n\
        HKEY_CURRENT_USER\\Software\\OurStick\\NavigationPane\\0123456789abcdef\r\n\
        \x20   {AB}    REG_SZ    00ff\r\n\
        \x20   other    REG_DWORD    0x1\r\n\
        \x20   with  spaces    REG_SZ    a b\r\n\
        \r\n";
    assert_eq!(
        listed_in(output),
        [
            ("{AB}".to_owned(), "00ff".to_owned()),
            ("with  spaces".to_owned(), "a b".to_owned())
        ]
    );
    assert!(listed_in("").is_empty());
}

#[test]
fn only_our_entries_of_sticks_gone_are_left_behind() {
    let (kept, gone, other) = (stick(1), stick(2), stick(3));
    let hex = crate::state::stick_hex;
    let listed = vec![
        (class_id(&kept), hex(&kept)),
        (class_id(&gone), hex(&gone)),
        // The name is not the class id of the stick it holds.
        (class_id(&other), hex(&gone)),
        (
            "{0E5AAE11-A475-4c5b-AB00-C66DE400274E}".to_owned(),
            hex(&gone),
        ),
        ("anything".to_owned(), "not a stick".to_owned()),
    ];
    assert_eq!(
        left_behind(&listed, &[kept]),
        LeftBehind {
            entries: vec![class_id(&gone)],
            odd: vec![
                class_id(&other),
                "{0E5AAE11-A475-4c5b-AB00-C66DE400274E}".to_owned(),
                "anything".to_owned()
            ],
        }
    );
    assert_eq!(left_behind(&listed[..1], &[kept]), LeftBehind::default());
}
