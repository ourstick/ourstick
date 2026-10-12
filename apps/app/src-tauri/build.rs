fn main() {
    copyright();
    tauri_build::build()
}

/// The bundles' copyright line, from tauri.conf.json, for About (ADR 0024):
/// Tauri keeps it for the installers but leaves it out of the app's own
/// configuration. One place for it still.
fn copyright() {
    println!("cargo:rerun-if-changed=tauri.conf.json");
    let config = std::fs::read_to_string("tauri.conf.json").expect("tauri.conf.json");
    let line = config
        .lines()
        .find_map(|line| line.trim().strip_prefix("\"copyright\": \""))
        .and_then(|rest| rest.strip_suffix("\",").or(rest.strip_suffix('"')))
        .expect("tauri.conf.json has a bundle copyright line");
    assert!(!line.contains('\\'), "the copyright line has no escapes");
    println!("cargo:rustc-env=OURSTICK_COPYRIGHT={line}");
}
