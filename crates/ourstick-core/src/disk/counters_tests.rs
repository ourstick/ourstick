use super::*;

#[test]
fn a_stat_line_gives_operations_and_bytes() {
    // A kernel of today: 17 fields (discards and flushes after the writes).
    let line = "  145933   26453 15328402   206480   52107   41255  3640968  1043020        0   458632  1301052   12   0  48   0  4560  51552\n";
    let counters = parse_stat(line).unwrap();
    assert_eq!(counters.ops, 145_933 + 52_107);
    assert_eq!(counters.read_bytes, 15_328_402 * 512);
    assert_eq!(counters.written_bytes, 3_640_968 * 512);
    // An old one: 11 fields.
    assert!(parse_stat("1 2 3 4 5 6 7 8 9 10 11").is_some());
    assert!(parse_stat("1 2 3").is_none());
    assert!(parse_stat("1 2 x 4 5 6 7 8 9 10 11").is_none());
    assert!(parse_stat("").is_none());
}

#[test]
fn a_device_number_is_split_as_glibc_does() {
    // 8:1 (sda1), 253:0 (dm-0), and a minor over 255.
    assert_eq!(major_minor(0x801), (8, 1));
    assert_eq!(major_minor(253 << 8), (253, 0));
    let big: u64 = ((259 & 0xfff) << 8) | (300 & 0xff) | ((300 & !0xff) << 12);
    assert_eq!(major_minor(big), (259, 300));
}

#[test]
fn counters_since_are_differences_and_never_go_back() {
    let before = Counters {
        ops: 10,
        read_bytes: 1000,
        written_bytes: 50,
    };
    let after = Counters {
        ops: 15,
        read_bytes: 3000,
        written_bytes: 40,
    };
    let since = before.until(after);
    assert_eq!(
        since,
        Counters {
            ops: 5,
            read_bytes: 2000,
            written_bytes: 0,
        }
    );
    let mut sum = since;
    sum.add(since);
    assert_eq!(sum.read_bytes, 4000);
}

/// A fake `/sys`: `dev/block/M:N` links to `devices/...` folders, each with
/// a `stat`, partitions with a `partition` file, layers with `slaves/`.
#[cfg(unix)]
fn fake_sys() -> tempfile::TempDir {
    use std::os::unix::fs::symlink;
    let sys = tempfile::tempdir().unwrap();
    let root = sys.path();
    let disk = root.join("devices/pci/sda");
    let part = disk.join("sda1");
    let crypt = root.join("devices/virtual/block/dm-0");
    let raid = root.join("devices/virtual/block/md0");
    for (folder, sectors) in [(&disk, 1000), (&part, 600), (&crypt, 500), (&raid, 7)] {
        std::fs::create_dir_all(folder).unwrap();
        std::fs::write(
            folder.join("stat"),
            format!("1 0 {sectors} 0 2 0 {sectors} 0 0 0 0\n"),
        )
        .unwrap();
    }
    std::fs::write(part.join("partition"), "1\n").unwrap();
    std::fs::create_dir_all(crypt.join("slaves")).unwrap();
    symlink(&part, crypt.join("slaves/sda1")).unwrap();
    std::fs::create_dir_all(raid.join("slaves")).unwrap();
    symlink(&part, raid.join("slaves/sda1")).unwrap();
    symlink(&disk, raid.join("slaves/sdb1")).unwrap();
    std::fs::create_dir_all(root.join("dev/block")).unwrap();
    symlink(&part, root.join("dev/block/8:1")).unwrap();
    symlink(&disk, root.join("dev/block/8:0")).unwrap();
    symlink(&crypt, root.join("dev/block/253:0")).unwrap();
    symlink(&raid, root.join("dev/block/9:0")).unwrap();
    sys
}

#[cfg(target_os = "linux")]
#[test]
fn a_device_is_followed_down_to_its_disk() {
    let sys = fake_sys();
    let counters = SystemCounters::under(sys.path());
    let disk_bytes = 1000 * 512;
    // A partition: its disk. A LUKS layer over it: the same disk.
    for (major, minor) in [(8, 1), (8, 0), (253, 0)] {
        let dev = (major << 8) | minor;
        let read = counters.read(&DiskId::Device(dev)).unwrap();
        assert_eq!(read.read_bytes, disk_bytes, "{major}:{minor}");
    }
    // Over two devices: its own line.
    let raid = counters.read(&DiskId::Device(9 << 8)).unwrap();
    assert_eq!(raid.read_bytes, 7 * 512);
    // Unknown, anonymous, or not a device: nothing.
    assert!(counters.read(&DiskId::Device((7 << 8) | 3)).is_none());
    assert!(counters.read(&DiskId::Device(42)).is_none());
    assert!(counters.read(&DiskId::Unknown).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn the_real_system_answers_for_this_disk() {
    use std::os::unix::fs::MetadataExt;
    let dev = std::fs::metadata(env!("CARGO_MANIFEST_DIR")).unwrap().dev();
    // Not every filesystem has a line (btrfs): only checked when it does.
    if let Some(now) = SystemCounters::default().read(&DiskId::Device(dev)) {
        assert!(now.ops > 0);
    }
}

#[test]
fn no_counters_never_count() {
    assert_eq!(NoCounters.read(&DiskId::Device(0x801)), None);
}
