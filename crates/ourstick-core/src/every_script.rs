//! Names and paths in every script never panic: whatever a user types or a
//! peer sends goes through these functions, and a slice at a byte offset
//! inside a character would crash the program (it did, for a two-letter
//! Greek name). Each sample is tried whole and cut at every length.

use crate::download::files::{check_manifest_paths, component_problem, is_reserved_device};
use crate::download::saved_names::safe_name;
use crate::manifest::{name_for_manifest, validate_name, Entry, Manifest};
use crate::places::comparable;
use crate::stick_names::{folder_name, name_from_folder, new_stick_name, normal_form, same_name};
use crate::untrusted::escape;

/// Words in many scripts, with combining marks, right-to-left text,
/// joined emoji, and characters of 2, 3 and 4 bytes.
const SAMPLES: &[&str] = &[
    "Φω",
    "αβ.txt",
    "Μαρι\u{301}α",
    "éé",
    "a€",
    "a中",
    "中文资料",
    "日本語のファイル",
    "한국어",
    "العربية",
    "עברית",
    "हिन्दी",
    "ภาษาไทย",
    "Кириллица",
    "👨\u{200d}👩\u{200d}👧",
    "🏳\u{fe0f}\u{200d}🌈",
    "e\u{301}\u{301}",
    "COM¹",
    "LPT²",
    "ＣＯＭ１",
    "Ω.Ω",
    "\u{202e}gnp.exe",
    "𝔘𝔫𝔦𝔠𝔬𝔡𝔢",
];

/// `sample` cut after each of its first eight characters, and whole.
fn cuts(sample: &str) -> Vec<String> {
    let mut cuts: Vec<String> = (1..=8)
        .map(|count| sample.chars().take(count).collect())
        .collect();
    cuts.push(sample.to_owned());
    cuts.push(format!("{sample}.{sample}"));
    cuts
}

#[test]
fn names_and_paths_in_every_script_never_panic() {
    for sample in SAMPLES {
        for text in cuts(sample) {
            let _ = component_problem(&text, true);
            let _ = component_problem(&text, false);
            let _ = is_reserved_device(&text);
            let _ = validate_name(&text);
            let _ = name_for_manifest(&text);
            let _ = escape(&text);
            let _ = normal_form(&text);
            let _ = same_name(&text, sample);
            let _ = new_stick_name(&text, &[]);
            assert!(!name_from_folder(&text).is_empty());
            let folder = folder_name(&text, |_| false);
            assert!(
                component_problem(&folder, true).is_none(),
                "{text:?} gave the folder {folder:?}"
            );
            let _ = comparable(std::path::Path::new(&format!("/nowhere/{text}")));
            let manifest = Manifest {
                entries: vec![Entry {
                    path: format!("{text}/{text}"),
                    size: 1,
                    mtime: 0,
                    hash: [0; 32],
                }],
                ..Manifest::default()
            };
            let _ = check_manifest_paths(&manifest);
            if !text.is_empty() {
                for windows in [true, false] {
                    let saved = safe_name(&text, windows);
                    assert!(
                        component_problem(&saved, windows).is_none(),
                        "{text:?} saved as {saved:?}"
                    );
                    assert_eq!(safe_name(&saved, windows), saved, "{text:?}");
                }
                let path = format!("{text}/{text}");
                for (windows, fold_case) in [(true, true), (false, false), (false, true)] {
                    let rules = crate::download::saved_paths::Rules {
                        windows,
                        fold_case,
                        fold_normal: true,
                    };
                    let saved = crate::download::saved_paths::saved_paths(
                        &[&path, &text],
                        &[],
                        &Default::default(),
                        |_, _| crate::download::saved_paths::OnDisk::Nothing,
                        rules,
                    );
                    assert_eq!(saved.saved.len(), 2);
                    for place in &saved.saved {
                        for name in place.split('/') {
                            assert!(component_problem(name, windows).is_none(), "{place:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn reserved_devices_are_whole_names_only() {
    for reserved in [
        "CON",
        "con.txt",
        "COM1",
        "lpt9.log",
        "COM¹",
        "LPT³",
        "NUL ",
        "COM0",
        "lpt0.txt",
        "CONIN$",
        "conout$.log",
    ] {
        assert!(is_reserved_device(reserved), "{reserved}");
    }
    for fine in ["COM10", "Φω", "αβ.txt", "CONSOLE", "ＣＯＭ１", "COMa"] {
        assert!(!is_reserved_device(fine), "{fine}");
    }
}
