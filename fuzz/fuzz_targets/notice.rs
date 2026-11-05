//! `version.json` as any server (or anyone in the way) may send it: never a
//! panic; without our keys' signature nothing is accepted.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::new_version::{read_notice, NoticeKeys, System};

fuzz_target!(|data: &[u8]| {
    let keys = NoticeKeys::ourstick();
    for system in [System::Windows, System::Macos, System::Linux] {
        let _ = read_notice(data, &keys, system);
    }
});
