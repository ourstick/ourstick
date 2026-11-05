//! A signed manifest record from any peer: never a panic; a record read is
//! the same bytes written back; checking it never panics, and a record not
//! signed by the stick's key is never accepted.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::{ManifestHint, SignedManifestRecord, StickId, StickSigningKey};

fuzz_target!(|data: &[u8]| {
    let Ok(record) = SignedManifestRecord::from_bytes(data) else {
        return;
    };
    assert_eq!(&record.to_bytes()[..], data);
    let hint = ManifestHint {
        seq: 0,
        hash: [0; 32],
    };
    // A stick whose key signed nothing the fuzzer has (the seeds are signed
    // with [7; 32]): no record is ever accepted for it.
    let ours = StickSigningKey::from_bytes(&[8; 32]).stick_id();
    assert!(record.verify(&ours, &hint, None).is_err());
    // The stick the record names.
    let named: [u8; 32] = data[1..33].try_into().expect("32 bytes");
    if let Ok(named) = StickId::from_bytes(&named) {
        let _ = record.verify(&named, &hint, None);
        let _ = record.verify(&named, &hint, Some(u64::MAX));
    }
});
