//! Places in a folder the user changed (ADR 0010, rules 3 and 9).

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

fn plan(paths: &[&str], rules: Rules) -> SavedPaths {
    saved_paths(paths, &[], |_, _| OnDisk::Nothing, rules)
}

#[test]
fn numbering_ends_and_the_file_is_left_out() {
    // Every place under `Trip` is in the way (it is a link elsewhere now).
    let on_disk = |place: &str, _: Option<usize>| {
        if place.starts_with("Trip/") {
            OnDisk::File
        } else {
            OnDisk::Nothing
        }
    };
    let recorded = [Recorded {
        path: "Trip/a.jpg",
        saved_as: "Trip/a.jpg",
    }];
    let saved = saved_paths(
        &["Trip/A.jpg", "Trip/a.jpg", "b"],
        &recorded,
        on_disk,
        WINDOWS,
    );
    assert_eq!(saved.left_out, [0]);
    assert_eq!(saved.saved[2], "b");
    assert!(saved.renamed.iter().all(|line| line.path != "Trip/A.jpg"));
}

#[test]
fn a_recorded_folder_that_is_now_a_file_moves_its_files() {
    let on_disk = |place: &str, _: Option<usize>| {
        if place == "Trip" {
            OnDisk::File
        } else {
            OnDisk::Nothing
        }
    };
    let recorded = [Recorded {
        path: "Trip/a.jpg",
        saved_as: "Trip/a.jpg",
    }];
    let saved = saved_paths(&["Trip/a.jpg"], &recorded, on_disk, LINUX);
    assert_eq!(saved.saved, ["Trip (2)/a.jpg"]);
    assert_eq!(saved.displaced, ["Trip/a.jpg"]);
}

#[test]
fn a_long_ourstick_name_is_marked_and_still_fits() {
    let long = format!(".ourstick-{}", "x".repeat(245));
    assert_eq!(long.len(), 255);
    let saved = plan(&[&long], LINUX);
    let name = &saved.saved[0];
    assert!(
        name.starts_with("_.ourstick-") && name.len() <= 255,
        "{name}"
    );
    assert!(saved.left_out.is_empty());
}
