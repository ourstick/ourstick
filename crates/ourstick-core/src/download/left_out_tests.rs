use super::*;

fn set(paths: &[&str]) -> LeftOut {
    LeftOut::new(
        &paths
            .iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>(),
    )
}

fn strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| path.to_string()).collect()
}

#[test]
fn a_path_or_a_folder_it_lies_in_covers_it() {
    let left = set(&["Films", "Disk/MUSIC"]);
    assert_eq!(left.covers("Films"), Some("Films"));
    assert_eq!(left.covers("Films/a/b.mkv"), Some("Films"));
    assert_eq!(left.covers("Disk/MUSIC/x.flac"), Some("Disk/MUSIC"));
    assert_eq!(left.covers("Disk/MUSIC"), Some("Disk/MUSIC"));
    // A name that only begins the same is another name.
    assert_eq!(left.covers("Films old/a"), None);
    assert_eq!(left.covers("Disk/MUSICAL/x"), None);
    assert_eq!(left.covers("Disk/Pictures/p.jpg"), None);
    assert_eq!(left.covers("Disk"), None);
    assert!(LeftOut::default().covers("anything").is_none());
}

#[test]
fn ticks_show_on_off_and_part() {
    let left = set(&["Films", "Disk/MUSIC"]);
    assert_eq!(left.tick_of("Films"), Tick::Off);
    assert_eq!(left.tick_of("Films/a"), Tick::Off);
    assert_eq!(left.tick_of("Disk"), Tick::Part);
    assert_eq!(left.tick_of("Disk/MUSIC"), Tick::Off);
    assert_eq!(left.tick_of("Disk/Pictures"), Tick::On);
    assert_eq!(left.tick_of("Music"), Tick::On);
}

#[test]
fn ticking_off_a_folder_covers_its_inside() {
    let mut left = set(&["Disk/MUSIC", "Disk/Pictures", "Other"]);
    left.tick("Disk", false, &[]);
    assert_eq!(left.paths(), strings(&["Disk", "Other"]));
    // On again: everything inside comes.
    left.tick("Disk", true, &[]);
    assert_eq!(left.paths(), strings(&["Other"]));
}

#[test]
fn ticking_one_thing_on_inside_a_folder_that_is_off() {
    let mut left = set(&["Disk"]);
    let inside = strings(&["Disk/MUSIC", "Disk/Pictures", "Disk/new in"]);
    left.tick("Disk/new in", true, &inside);
    // The folder is on, the rest of it off one by one: what the sender adds
    // to it later comes.
    assert_eq!(left.paths(), strings(&["Disk/MUSIC", "Disk/Pictures"]));
    assert_eq!(left.tick_of("Disk"), Tick::Part);
    assert_eq!(left.covers("Disk/added later/x"), None);
    // Off and on inside a folder that is on.
    left.tick("Disk/new in", false, &inside);
    left.tick("Disk/MUSIC", true, &inside);
    assert_eq!(left.paths(), strings(&["Disk/Pictures", "Disk/new in"]));
}

#[test]
fn a_set_to_save_is_checked_and_simplified() {
    let saved = LeftOut::normalised(&strings(&["Disk/MUSIC", "Disk", "Films", "Other/x"])).unwrap();
    assert_eq!(saved.paths(), strings(&["Disk", "Films", "Other/x"]));
    assert_eq!(LeftOut::normalised(&[]).unwrap(), LeftOut::default());
    for (bad, why) in [
        ("a/b/c", PathProblem::TooDeep),
        ("", PathProblem::NotAName),
        ("/a", PathProblem::NotAName),
        ("a/", PathProblem::NotAName),
        ("a/..", PathProblem::NotAName),
        (".", PathProblem::NotAName),
        ("a\\b", PathProblem::BadCharacter),
        ("a\u{0}", PathProblem::BadCharacter),
        ("a/b\u{1b}", PathProblem::BadCharacter),
    ] {
        assert_eq!(LeftOut::normalised(&strings(&[bad])), Err(why), "{bad:?}");
    }
}

#[test]
fn only_what_is_in_the_folder_is_taken_for_its_inside() {
    let mut left = set(&["Disk"]);
    let given = strings(&[
        "Disk/a",
        "Disk/b",
        "Films",
        "Other/x",
        "Disk/a/deeper",
        "Diskette/y",
    ]);
    left.tick("Disk/a", true, &given);
    assert_eq!(left.paths(), strings(&["Disk/b"]));
}

#[test]
fn rows_a_check_now_refuses_are_dropped_not_fatal() {
    let rows = strings(&["Films", "a\u{1b}", "Disk/MUSIC", "x/y/z"]);
    assert_eq!(
        LeftOut::valid_only(&rows),
        strings(&["Films", "Disk/MUSIC"])
    );
    assert!(LeftOut::normalised(&LeftOut::valid_only(&rows)).is_ok());
    assert!(LeftOut::default().is_empty() && !set(&["Films"]).is_empty());
}
