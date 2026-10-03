//! The ticks' rules for the windows (ADR 0012, Amendment 1): the core's
//! `LeftOut` decides; a window keeps the set of paths left out and asks here
//! for each change, so one place decides.

use ourstick_core::download::LeftOut;

use super::choice::tick_word;

/// The set after ticking `path` on or off; `siblings` are the paths beside
/// it in its folder (itself too).
#[tauri::command]
pub fn choice_tick(
    left_out: Vec<String>,
    path: String,
    on: bool,
    siblings: Vec<String>,
) -> Vec<String> {
    let mut set = LeftOut::new(&left_out);
    set.tick(&path, on, &siblings);
    set.paths()
}

/// How each of `paths` shows with `left_out`: "on", "off" or "part".
#[tauri::command]
pub fn choice_ticks(left_out: Vec<String>, paths: Vec<String>) -> Vec<&'static str> {
    let set = LeftOut::new(&left_out);
    paths
        .iter()
        .map(|path| tick_word(set.tick_of(path)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| path.to_string()).collect()
    }

    #[test]
    fn the_window_asks_the_core_for_each_tick() {
        let inside = strings(&["Disk/MUSIC", "Disk/Pictures"]);
        let set = choice_tick(Vec::new(), "Disk/MUSIC".to_owned(), false, inside.clone());
        assert_eq!(set, strings(&["Disk/MUSIC"]));
        assert_eq!(
            choice_ticks(
                set.clone(),
                strings(&["Disk", "Disk/MUSIC", "Disk/Pictures"])
            ),
            ["part", "off", "on"]
        );
        let set = choice_tick(set, "Disk".to_owned(), false, Vec::new());
        assert_eq!(set, strings(&["Disk"]));
        let set = choice_tick(set, "Disk/Pictures".to_owned(), true, inside);
        assert_eq!(set, strings(&["Disk/MUSIC"]));
    }
}
