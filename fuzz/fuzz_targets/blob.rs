//! A blob from a sender: decrypting never panics and never writes more than
//! it reads. Plaintext encrypts and decrypts back to itself.
#![no_main]

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use ourstick_core::{decrypt_blob, encrypt_blob, ChaChaCipher, Cipher, NoCipher, StickKey};

fuzz_target!(|data: &[u8]| {
    let chacha = ChaChaCipher::new(&StickKey::from_bytes([9; 32]));
    let ciphers: [&dyn Cipher; 2] = [&chacha, &NoCipher];
    for cipher in ciphers {
        let mut out = Vec::new();
        if let Ok(written) = decrypt_blob(cipher, &mut Cursor::new(data), &mut out) {
            assert_eq!(written as usize, out.len());
            assert!(out.len() <= data.len());
        }

        let mut blob = Vec::new();
        encrypt_blob(cipher, &mut Cursor::new(data), &mut blob).expect("encrypts");
        let mut back = Vec::new();
        decrypt_blob(cipher, &mut Cursor::new(&blob), &mut back).expect("decrypts");
        assert_eq!(back, data);
    }
});
