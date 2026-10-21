use super::*;

/// A home with a dot-folder, a main folder and OurStick's data folder, a
/// system folder and a disk beside it, all in a temporary folder.
struct Device {
    _dir: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    rules: SendRules,
}

fn device() -> Device {
    let dir = temporary_folder();
    let root = dir.path().to_path_buf();
    let home = root.join("home/alice");
    for folder in [
        "home/alice/.ssh/keys",
        "home/alice/Library/Mail",
        "home/alice/Documents/Bank",
        "home/alice/Pictures/Holiday 2025/Summer",
        "home/alice/data/ourstick",
        "usr/lib",
        "run/media/alice/Expansion",
        "Expansion/Photos",
    ] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    let rules = SendRules {
        home: home.clone(),
        private: vec![home.join("Library")],
        system: vec![root.join("usr"), root.join("run")],
        not_system: vec![root.join("run/media")],
        main_folders: vec![home.join("Documents"), home.join("Pictures")],
        own: vec![Taken {
            place: comparable(&home.join("data/ourstick")),
            what: "OurStick's data folder".to_owned(),
        }],
        chosen: Vec::new(),
    };
    Device {
        _dir: dir,
        root,
        home,
        rules,
    }
}

/// On Windows %TEMP% lies in AppData, which is hidden: what stands for
/// places outside the home is made in the build folder instead.
#[cfg(windows)]
fn temporary_folder() -> tempfile::TempDir {
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    tempfile::tempdir_in(target).unwrap()
}

#[cfg(not(windows))]
fn temporary_folder() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn refusal(device: &Device, path: &Path) -> String {
    match may_send(path, &device.rules) {
        MaySend::Refuse(why) => why,
        other => panic!("{}: {other:?}", path.display()),
    }
}

#[test]
fn a_folder_or_a_file_inside_the_home_is_fine() {
    let device = device();
    let summer = device.home.join("Pictures/Holiday 2025/Summer");
    assert_eq!(may_send(&summer, &device.rules), MaySend::Fine);
    let notes = device.home.join("Documents/Bank/notes.txt");
    std::fs::write(&notes, "x").unwrap();
    assert_eq!(may_send(&notes, &device.rules), MaySend::Fine);
}

#[test]
fn the_home_and_what_holds_it_are_refused() {
    let device = device();
    let words = "Your whole home folder cannot be sent. Choose folders inside it.";
    for path in [&device.home, &device.root.join("home"), &device.root] {
        assert_eq!(refusal(&device, path), words, "{}", path.display());
    }
}

#[test]
fn private_folders_and_what_is_inside_them_are_refused() {
    let device = device();
    for path in [
        device.home.join(".ssh"),
        device.home.join(".ssh/keys"),
        // Written with `..`: compared where it really is.
        device.home.join("Documents/../.ssh"),
    ] {
        assert_eq!(
            refusal(&device, &path),
            "\u{201c}.ssh\u{201d} holds private settings and cannot be sent.",
            "{}",
            path.display()
        );
    }
    assert_eq!(
        refusal(&device, &device.home.join("Library/Mail")),
        "\u{201c}Library\u{201d} holds private settings and cannot be sent."
    );
}

#[test]
fn the_systems_folders_are_refused_but_its_disks_are_not() {
    let device = device();
    assert_eq!(
        refusal(&device, &device.root.join("usr/lib")),
        "\u{201c}usr\u{201d} is part of the system and cannot be sent."
    );
    assert_eq!(
        refusal(&device, &device.root.join("run")),
        "\u{201c}run\u{201d} is part of the system and cannot be sent."
    );
    let disk = device.root.join("run/media/alice/Expansion");
    assert_eq!(may_send(&disk, &device.rules), MaySend::Fine);
}

#[test]
fn oursticks_own_folders_are_refused() {
    let device = device();
    assert_eq!(
        refusal(&device, &device.home.join("data/ourstick")),
        "\u{201c}ourstick\u{201d} is OurStick's own folder and cannot be sent."
    );
    assert_eq!(
        refusal(&device, &device.home.join("data")),
        "\u{201c}data\u{201d} holds OurStick's own folder and cannot be sent."
    );
}

#[test]
fn a_main_folder_of_the_home_is_asked_about() {
    let device = device();
    assert_eq!(
        may_send(&device.home.join("Documents"), &device.rules),
        MaySend::Ask(Ask::MainFolder {
            name: "Documents".to_owned()
        })
    );
    assert_eq!(
        Ask::MainFolder {
            name: "Documents".to_owned()
        }
        .question(),
        "Send all of \u{201c}Documents\u{201d}?"
    );
    // A folder inside one is not.
    let bank = device.home.join("Documents/Bank");
    assert_eq!(may_send(&bank, &device.rules), MaySend::Fine);
}

#[test]
fn what_is_in_the_list_already_is_refused_inside_around_or_again() {
    let mut device = device();
    let holiday = device.home.join("Pictures/Holiday 2025");
    let summer = holiday.join("Summer");
    device.rules.chosen = vec![holiday.clone()];
    assert_eq!(
        refusal(&device, &summer),
        "\u{201c}Summer\u{201d} is inside \u{201c}Holiday 2025\u{201d}, so it comes along already."
    );
    assert_eq!(
        refusal(&device, &holiday),
        "\u{201c}Holiday 2025\u{201d} is in the list already."
    );
    device.rules.chosen = vec![summer];
    assert_eq!(
        refusal(&device, &holiday),
        "\u{201c}Holiday 2025\u{201d} holds \u{201c}Summer\u{201d}, which is in the list already. Take \u{201c}Summer\u{201d} out first."
    );
}

#[test]
fn a_missing_or_relative_path_is_refused() {
    let device = device();
    assert_eq!(
        refusal(&device, &device.root.join("gone")),
        "\u{201c}gone\u{201d} is not there any more."
    );
    assert!(matches!(
        may_send(Path::new("Pictures"), &device.rules),
        MaySend::Refuse(_)
    ));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_is_refused_even_to_a_fine_folder() {
    let device = device();
    let link = device.root.join("Expansion/to-summer");
    std::os::unix::fs::symlink(device.home.join("Pictures/Holiday 2025/Summer"), &link).unwrap();
    assert_eq!(
        refusal(&device, &link),
        "\u{201c}to-summer\u{201d} is a shortcut. Choose what it points to instead."
    );
}

#[test]
fn a_whole_disk_is_asked_about() {
    assert_eq!(
        Ask::WholeDisk {
            name: "Expansion".to_owned()
        }
        .question(),
        "Send the whole disk \u{201c}Expansion\u{201d}?"
    );
    // A folder in a temporary folder is not a disk of its own.
    let dir = tempfile::tempdir().unwrap();
    assert!(!is_whole_disk(dir.path()));
}

#[cfg(target_os = "linux")]
#[test]
fn a_mount_point_is_a_whole_disk() {
    // /proc is its own file system wherever Linux runs.
    assert!(is_whole_disk(Path::new("/proc")));
}

#[test]
fn this_devices_rules_refuse_its_system() {
    let home = std::env::temp_dir().join("alice");
    let rules = SendRules::of_this_device(&home, Vec::new(), Vec::new());
    #[cfg(target_os = "linux")]
    {
        assert!(rules.system.contains(&PathBuf::from("/etc")));
        assert!(rules.not_system.contains(&PathBuf::from("/run/media")));
    }
    #[cfg(target_os = "macos")]
    assert!(rules.private.contains(&home.join("Library")));
    #[cfg(windows)]
    assert!(rules.private.contains(&home.join("AppData")));
    assert!(!rules.system.is_empty());
}

#[cfg(windows)]
#[test]
fn a_hidden_folder_counts_inside_the_home_only() {
    let hide = |path: &Path| {
        let done = std::process::Command::new("attrib")
            .arg("+h")
            .arg(path)
            .status()
            .unwrap();
        assert!(done.success());
    };
    let device = device();
    // What holds the home is hidden (as AppData holds %TEMP%): it does not count.
    hide(&device.root.join("home"));
    let summer = device.home.join("Pictures/Holiday 2025/Summer");
    assert_eq!(may_send(&summer, &device.rules), MaySend::Fine);
    hide(&device.home.join("Pictures/Holiday 2025"));
    assert!(refusal(&device, &summer).contains("Holiday 2025"));
}
