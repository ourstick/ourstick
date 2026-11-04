use super::*;
use crate::manifest::Entry;

fn manifest(paths: &[(&str, u64)]) -> Manifest {
    Manifest {
        entries: paths
            .iter()
            .map(|(path, size)| Entry {
                path: (*path).to_owned(),
                size: *size,
                mtime: 0,
                hash: [0; 32],
            })
            .collect(),
        ..Manifest::default()
    }
}

#[test]
fn what_is_in_one_folder_is_named_by_its_whole_path() {
    let stick = manifest(&[
        ("Disk/MUSIC/a.flac", 4),
        ("Disk/MUSIC/deeper/b.flac", 2),
        ("Disk/readme.txt", 1),
        ("Diskette/x", 9),
        ("Films/a.mkv", 7),
    ]);
    let top = entries_in(&stick, None, None);
    let names: Vec<&str> = top.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["Disk", "Diskette", "Films"]);
    let inside = entries_in(&stick, Some("Disk"), None);
    let named: Vec<(&str, bool, u64, u64)> = inside
        .iter()
        .map(|entry| (entry.name.as_str(), entry.is_dir, entry.files, entry.bytes))
        .collect();
    assert_eq!(
        named,
        [("Disk/MUSIC", true, 2, 6), ("Disk/readme.txt", false, 1, 1)]
    );
    // What a preview says each would write.
    let to_write: BTreeMap<String, u64> = [("Disk/MUSIC".to_owned(), 2)].into();
    let inside = entries_in(&stick, Some("Disk"), Some(&to_write));
    assert_eq!(inside[0].to_write, 2);
    assert_eq!(inside[1].to_write, 0);
    assert!(entries_in(&stick, Some("Nothing"), None).is_empty());
    // A new folder: no plan, every file written.
    let all = all_bytes_by_path(&stick);
    assert_eq!(all.get("Disk"), Some(&7));
    assert_eq!(all.get("Disk/MUSIC"), Some(&6));
    assert_eq!(all.get("Films/a.mkv"), Some(&7));
}

#[test]
fn a_stick_that_is_not_encrypted_is_refused() {
    use crate::blob::encrypt_blob;
    use crate::cipher::{ChaChaCipher, NoCipher, StickKey};
    use crate::{AddressHint, ManifestHint, NodeId, StickSigningKey};
    let ticket = Ticket {
        stick_id: StickSigningKey::from_bytes(&[1; 32]).stick_id(),
        node_id: NodeId([2; 32]),
        manifest_hint: ManifestHint {
            seq: 1,
            hash: [0; 32],
        },
        address_hints: vec![AddressHint::Ipv4 {
            ip: [127, 0, 0, 1],
            port: 1,
        }],
        stick_key: StickKey::from_bytes([3; 32]),
    };
    let blob = |cipher: &dyn Cipher| {
        let mut blob = Vec::new();
        encrypt_blob(cipher, &mut std::io::Cursor::new(b"manifest"), &mut blob).unwrap();
        blob
    };
    let refused = cipher_of(&blob(&NoCipher), &ticket).err().unwrap();
    assert!(
        matches!(refused, DownloadError::Cipher(CipherError::NotEncrypted)),
        "{refused:?}"
    );
    let chacha = ChaChaCipher::new(&ticket.stick_key);
    assert_eq!(
        cipher_of(&blob(&chacha), &ticket).unwrap().id(),
        CipherId::XChaCha20Poly1305
    );
}
