//! A reader's report. A reader holds the stick key, so it can seal any
//! report: the bytes are read as a report, sealed and opened again (the
//! owner's side), which must give back the same report or refuse it. Raw
//! bytes are opened too: never a panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::report::{open, seal, ReaderKey, ReaderReport};
use ourstick_core::{StickKey, StickSigningKey};

fuzz_target!(|data: &[u8]| {
    let stick_id = StickSigningKey::from_bytes(&[7; 32]).stick_id();
    let stick_key = StickKey::from_bytes([9; 32]);
    let _ = open(data, &stick_id, &stick_key);

    let Ok(report) = postcard::from_bytes::<ReaderReport>(data) else {
        return;
    };
    let reader = ReaderKey::from_bytes(&[3; 32]);
    let Ok(sealed) = seal(&report, &reader, &stick_id, &stick_key) else {
        return;
    };
    let opened = open(&sealed, &stick_id, &stick_key).expect("a sealed report opens");
    assert_eq!(opened.report, report);
    assert_eq!(opened.reader, reader.reader_id());
});
