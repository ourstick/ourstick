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

fn plan<'a>(paths: &[&'a str], rules: Rules) -> SavedPaths<'a> {
    saved_paths(
        paths,
        &[],
        &Default::default(),
        |_, _| OnDisk::Nothing,
        rules,
    )
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
        &Default::default(),
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
    let saved = saved_paths(
        &["Trip/a.jpg"],
        &recorded,
        &Default::default(),
        on_disk,
        LINUX,
    );
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

#[test]
fn what_is_not_chosen_takes_no_new_place_and_keeps_its_recorded_ones() {
    let not_chosen: std::collections::HashSet<usize> = [0, 1].into();
    // `Films?/a` came before (saved as `Films_/a`); `Films?/b` never did.
    let recorded = [Recorded {
        path: "Films?/a",
        saved_as: "Films_/a",
    }];
    // Meanwhile the sender added a genuine `Films_/c`.
    let saved = saved_paths(
        &["Films?/a", "Films?/b", "Films_/c"],
        &recorded,
        &not_chosen,
        |_, _| OnDisk::Nothing,
        WINDOWS,
    );
    // The un-ticked folder keeps its place; the genuine one is numbered.
    assert_eq!(saved.saved[0], "Films_/a");
    assert_eq!(saved.saved[2], "Films_ (2)/c");
    // Nothing of what is not got is told as renamed.
    assert!(saved
        .renamed
        .iter()
        .all(|line| !line.path.starts_with("Films?")));
    assert!(saved.left_out.is_empty());
}

#[test]
fn a_file_and_a_folder_of_one_name_with_one_not_chosen() {
    // The file `a` not got: the folder `a` of `a/b` takes the place.
    let not_file: std::collections::HashSet<usize> = [0].into();
    let saved = saved_paths(
        &["a", "a/b"],
        &[],
        &not_file,
        |_, _| OnDisk::Nothing,
        WINDOWS,
    );
    assert_eq!(saved.saved, ["a", "a/b"]);
    // Ticked again later, with `a/b` recorded: the file is numbered.
    let recorded = [Recorded {
        path: "a/b",
        saved_as: "a/b",
    }];
    let saved = saved_paths(
        &["a", "a/b"],
        &recorded,
        &Default::default(),
        |_, _| OnDisk::Nothing,
        WINDOWS,
    );
    assert_eq!(saved.saved, ["a (2)", "a/b"]);
}

#[test]
fn many_names_that_are_one_here_cost_a_bounded_number_of_tries() {
    // Every case of a ten-letter name: 1 024 names Windows takes for one.
    let names: Vec<String> = (0..1024u32)
        .map(|bits| {
            "abcdefghij"
                .chars()
                .enumerate()
                .map(|(at, c)| {
                    if bits & (1 << at) != 0 {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    }
                })
                .collect()
        })
        .collect();
    let mut paths: Vec<&str> = names.iter().map(String::as_str).collect();
    paths.sort_unstable();
    let started = std::time::Instant::now();
    let saved = plan(&paths, WINDOWS);
    // Up to the round's tries, each gets a number; the rest wait.
    let numbered = paths.len() - saved.left_out.len();
    assert!(numbered > 600 && !saved.left_out.is_empty(), "{numbered}");
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
}
