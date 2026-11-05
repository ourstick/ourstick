//! A manifest from a sender (who holds the stick key, so may encrypt any
//! bytes): never a panic, and a manifest accepted encodes and decodes again
//! to the same manifest.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ourstick_core::Manifest;

fuzz_target!(|data: &[u8]| {
    let Ok(manifest) = Manifest::from_bytes(data) else {
        return;
    };
    let bytes = manifest
        .to_bytes()
        .expect("a manifest accepted is a valid manifest");
    let again = Manifest::from_bytes(&bytes).expect("an encoded manifest decodes");
    assert_eq!(again, manifest);
});
