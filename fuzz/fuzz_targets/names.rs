//! A peer's text shown to the user: the escaped form holds no control or
//! direction-turning character, and stick names never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::report::name_is_allowed;
use ourstick_core::stick_names::{folder_name, name_from_folder, normal_form, same_name};
use ourstick_core::untrusted::escape;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let escaped = escape(text);
    assert!(!escaped.chars().any(|c| c.is_control()
        || ('\u{202a}'..='\u{202e}').contains(&c)
        || ('\u{2066}'..='\u{2069}').contains(&c)));
    let normal = normal_form(text);
    assert!(same_name(text, &normal));
    let _ = name_is_allowed(text);
    let from_folder = name_from_folder(text);
    let _ = folder_name(&from_folder, |name| name.len() % 3 == 0);
});
