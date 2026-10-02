use super::*;

const WINDOWS: Rules = Rules {
    windows: true,
    fold_case: true,
    fold_normal: false,
};
const LINUX: Rules = Rules {
    windows: false,
    fold_case: false,
    fold_normal: false,
};

fn empty(_: &str, _: Option<usize>) -> OnDisk {
    OnDisk::Nothing
}

fn plan(paths: &[&str], rules: Rules) -> SavedPaths {
    saved_paths(paths, &[], empty, rules)
}

#[test]
fn names_that_can_be_saved_stay() {
    let paths = ["a.jpg", "Trip/x.jpg", "Trip/y.jpg"];
    let saved = plan(&paths, WINDOWS);
    assert_eq!(saved.saved, paths);
    assert!(saved.renamed.is_empty());
}

#[test]
fn a_renamed_folder_holds_all_its_files() {
    let saved = plan(&["Trip?/x.jpg", "Trip?/y.jpg", "Trip?/in/z.jpg"], WINDOWS);
    assert_eq!(
        saved.saved,
        ["Trip_/x.jpg", "Trip_/y.jpg", "Trip_/in/z.jpg"]
    );
    assert_eq!(
        saved.renamed,
        [SavedAs {
            path: "Trip?".into(),
            saved_as: "Trip_".into(),
            files: 3
        }]
    );
}

#[test]
fn a_name_needing_no_change_keeps_its_place() {
    // `a?.jpg` comes first (byte order) but the genuine `a_.jpg` keeps it.
    let saved = plan(&["a?.jpg", "a_.jpg"], WINDOWS);
    assert_eq!(saved.saved, ["a_ (2).jpg", "a_.jpg"]);
}

#[test]
fn names_one_file_here_are_numbered_only_where_the_disk_folds_case() {
    let paths = ["Photo.jpg", "photo.jpg"];
    assert_eq!(plan(&paths, WINDOWS).saved, ["Photo.jpg", "photo (2).jpg"]);
    // A case-sensitive Linux folder keeps both as they are.
    assert_eq!(plan(&paths, LINUX).saved, paths);
}

#[test]
fn a_file_and_a_folder_of_one_name() {
    // `a` is a file, `A/b` needs `A` as a folder on a disk that folds case.
    let saved = plan(&["A/b", "a"], WINDOWS);
    assert_eq!(saved.saved, ["A/b", "a (2)"]);
}

#[test]
fn a_file_in_the_way_moves_the_folder() {
    let on_disk = |place: &str, _: Option<usize>| {
        if place == "a" {
            OnDisk::File
        } else {
            OnDisk::Nothing
        }
    };
    let saved = saved_paths(&["a/b"], &[], on_disk, LINUX);
    assert_eq!(saved.saved, ["a (2)/b"]);
}

#[test]
fn a_renamed_name_never_lands_on_a_file_there() {
    let on_disk = |place: &str, _: Option<usize>| {
        if place == "a_.jpg" {
            OnDisk::File
        } else {
            OnDisk::Nothing
        }
    };
    let saved = saved_paths(&["a?.jpg"], &[], on_disk, WINDOWS);
    assert_eq!(saved.saved, ["a_ (2).jpg"]);
    // Unless that file is this entry's, placed before a stop.
    let mine = |place: &str, entry: Option<usize>| {
        if place == "a_.jpg" && entry != Some(0) {
            OnDisk::File
        } else {
            OnDisk::Nothing
        }
    };
    assert_eq!(
        saved_paths(&["a?.jpg"], &[], mine, WINDOWS).saved,
        ["a_.jpg"]
    );
}

#[test]
fn recorded_places_stay_in_later_versions() {
    let recorded = [Recorded {
        path: "a?.jpg",
        saved_as: "a_.jpg",
    }];
    // A genuine `a_.jpg` arriving later does not take the recorded place.
    let saved = saved_paths(&["a?.jpg", "a_.jpg"], &recorded, empty, WINDOWS);
    assert_eq!(saved.saved, ["a_.jpg", "a_ (2).jpg"]);
    // A file added later to a renamed folder goes into it.
    let recorded = [Recorded {
        path: "Trip?/x",
        saved_as: "Trip_ (2)/x",
    }];
    let saved = saved_paths(&["Trip?/x", "Trip?/y"], &recorded, empty, WINDOWS);
    assert_eq!(saved.saved, ["Trip_ (2)/x", "Trip_ (2)/y"]);
}

#[test]
fn a_recorded_place_is_taken_even_when_its_file_left_the_stick() {
    let recorded = [Recorded {
        path: "old?.txt",
        saved_as: "new.txt",
    }];
    let saved = saved_paths(&["new.txt"], &recorded, empty, WINDOWS);
    assert_eq!(saved.saved, ["new (2).txt"]);
}

#[test]
fn recorded_places_made_one_by_folding_are_moved() {
    // Recorded on a case-sensitive disk; the folder is now on one that folds.
    let recorded = [
        Recorded {
            path: "Photo.jpg",
            saved_as: "Photo.jpg",
        },
        Recorded {
            path: "photo.jpg",
            saved_as: "photo.jpg",
        },
    ];
    let saved = saved_paths(&["Photo.jpg", "photo.jpg"], &recorded, empty, WINDOWS);
    assert_eq!(saved.saved, ["Photo.jpg", "photo (2).jpg"]);
    assert_eq!(
        saved.moved,
        [("photo.jpg".to_owned(), "photo (2).jpg".to_owned())]
    );
}

#[test]
fn ourstick_names_at_the_top_are_marked() {
    let saved = plan(&[".ourstick-partial/x", "sub/.ourstick-partial"], LINUX);
    assert_eq!(
        saved.saved,
        ["_.ourstick-partial/x", "sub/.ourstick-partial"]
    );
}

#[test]
fn every_place_is_savable_and_no_two_are_one() {
    let paths = [
        "A/b", "CON.txt", "Con.TXT", "a", "a?/b", "a_/b", "x:y", "c:z", "dot.", "dot", "Trip?/1",
        "Trip_/1", "trip_/1",
    ];
    for rules in [WINDOWS, LINUX] {
        let saved = plan(&paths, rules);
        let mut seen = std::collections::HashSet::new();
        for place in &saved.saved {
            for name in place.split('/') {
                assert!(
                    crate::download::files::component_problem(name, rules.windows).is_none(),
                    "{place}"
                );
            }
            assert!(seen.insert(rules.fold(place)), "two at {place} ({rules:?})");
        }
    }
}
