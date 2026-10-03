use super::*;
use crate::stick_id::StickSigningKey;

fn stick(seed: u8) -> (StickId, StickKey) {
    (
        StickSigningKey::from_bytes(&[seed; 32]).stick_id(),
        StickKey::from_bytes([seed; 32]),
    )
}

fn report() -> ReaderReport {
    ReaderReport {
        counter: 1_791_000_000_000,
        seq: 42,
        state: ReportState::Downloading { percent: 60 },
        everything: false,
        kind: ComputerKind::MacOs,
        name: Some("Μαρία".to_owned()),
    }
}

#[test]
fn a_report_opens_with_the_sticks_key_and_names_its_reader() {
    let (id, key) = stick(1);
    let reader = ReaderKey::generate();
    let sealed = seal(&report(), &reader, &id, &key).unwrap();
    assert!(sealed.len() <= MAX_SEALED_BYTES);
    let opened = open(&sealed, &id, &key).unwrap();
    assert_eq!(opened.reader, reader.reader_id());
    assert_eq!(opened.report, report());
    // Each state and kind travels.
    for state in [
        ReportState::UpToDate,
        ReportState::Waiting(WaitReason::NoRoom),
        ReportState::Paused,
        ReportState::Forgotten,
    ] {
        let told = ReaderReport {
            state,
            name: None,
            kind: ComputerKind::Windows,
            ..report()
        };
        let sealed = seal(&told, &reader, &id, &key).unwrap();
        assert_eq!(open(&sealed, &id, &key).unwrap().report, told);
    }
}

#[test]
fn the_same_report_sealed_twice_differs() {
    let (id, key) = stick(1);
    let reader = ReaderKey::generate();
    // A random nonce: nothing on the wire says two reports are the same.
    assert_ne!(
        seal(&report(), &reader, &id, &key).unwrap(),
        seal(&report(), &reader, &id, &key).unwrap()
    );
}

#[test]
fn another_key_or_another_stick_does_not_open_it() {
    let (id, key) = stick(1);
    let (other_id, other_key) = stick(2);
    let sealed = seal(&report(), &ReaderKey::generate(), &id, &key).unwrap();
    assert_eq!(open(&sealed, &id, &other_key), Err(ReportError::Sealed));
    // The StickId is bound in: the right key, another stick named.
    assert_eq!(open(&sealed, &other_id, &key), Err(ReportError::Sealed));
}

#[test]
fn a_changed_byte_anywhere_is_refused() {
    let (id, key) = stick(1);
    let sealed = seal(&report(), &ReaderKey::generate(), &id, &key).unwrap();
    for at in 0..sealed.len() {
        let mut changed = sealed.clone();
        changed[at] ^= 0x01;
        assert!(open(&changed, &id, &key).is_err(), "byte {at}");
    }
}

#[test]
fn a_reader_cannot_speak_for_another() {
    let (id, key) = stick(1);
    let honest = ReaderKey::generate();
    let liar = ReaderKey::generate();
    // The liar has the link (the key) and seals a report naming the honest
    // reader's id, signed with its own key.
    let body = postcard::to_allocvec(&report()).unwrap();
    let signature = liar.sign(&signed_message(&id, &body));
    let plain = [&honest.reader_id().to_bytes()[..], &signature, &body].concat();
    let nonce = [7u8; NONCE_LEN];
    let sealed = aead(&key)
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &associated(&id),
            },
        )
        .unwrap();
    let wire = [&[VERSION][..], &nonce, &sealed].concat();
    assert_eq!(open(&wire, &id, &key), Err(ReportError::BadSignature));
}

#[test]
fn names_and_percents_out_of_bounds_are_refused() {
    let (id, key) = stick(1);
    let reader = ReaderKey::generate();
    for name in [
        "",
        "   ",
        "a\u{1b}b",
        "a\u{202e}b",
        "a\u{2028}b",
        &"x".repeat(41),
    ] {
        let told = ReaderReport {
            name: Some(name.to_owned()),
            ..report()
        };
        assert_eq!(
            seal(&told, &reader, &id, &key),
            Err(ReportError::BadName),
            "{name:?}"
        );
    }
    assert!(name_is_allowed(&"ω".repeat(40)));
    let told = ReaderReport {
        state: ReportState::Downloading { percent: 100 },
        ..report()
    };
    assert_eq!(seal(&told, &reader, &id, &key), Err(ReportError::Malformed));
}

#[test]
fn bad_input_is_refused_without_a_panic() {
    let (id, key) = stick(1);
    assert_eq!(open(&[], &id, &key), Err(ReportError::Malformed));
    assert_eq!(open(&[VERSION; 40], &id, &key), Err(ReportError::Malformed));
    assert_eq!(
        open(&vec![VERSION; MAX_SEALED_BYTES + 1], &id, &key),
        Err(ReportError::TooBig)
    );
    let mut sealed = seal(&report(), &ReaderKey::generate(), &id, &key).unwrap();
    sealed[0] = 2;
    assert_eq!(open(&sealed, &id, &key), Err(ReportError::UnknownVersion));
    // Every length and many fillings: an error, never a panic.
    for length in 0..=MAX_SEALED_BYTES + 2 {
        for fill in [0u8, 1, 0xff] {
            let mut bytes = vec![fill; length];
            if let Some(first) = bytes.first_mut() {
                *first = VERSION;
            }
            assert!(open(&bytes, &id, &key).is_err());
        }
    }
}

#[test]
fn a_well_sealed_body_that_is_not_a_report_is_refused() {
    let (id, key) = stick(1);
    let reader = ReaderKey::generate();
    for body in [vec![], vec![0xff; 10], {
        let mut longer = postcard::to_allocvec(&report()).unwrap();
        longer.push(0);
        longer
    }] {
        let signature = reader.sign(&signed_message(&id, &body));
        let plain = [&reader.reader_id().to_bytes()[..], &signature, &body].concat();
        let nonce = [3u8; NONCE_LEN];
        let sealed = aead(&key)
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &associated(&id),
                },
            )
            .unwrap();
        let wire = [&[VERSION][..], &nonce, &sealed].concat();
        assert_eq!(open(&wire, &id, &key), Err(ReportError::Malformed));
    }
}

#[test]
fn the_longest_name_still_fits() {
    let (id, key) = stick(1);
    let told = ReaderReport {
        counter: u64::MAX,
        seq: u64::MAX,
        state: ReportState::Waiting(WaitReason::FolderMissing),
        name: Some("\u{10FFFF}".repeat(MAX_NAME_CHARS)),
        ..report()
    };
    let sealed = seal(&told, &ReaderKey::generate(), &id, &key).unwrap();
    assert!(sealed.len() <= MAX_SEALED_BYTES, "{}", sealed.len());
}

#[test]
fn a_reader_key_roundtrips_and_never_prints_itself() {
    let key = ReaderKey::generate();
    let again = ReaderKey::from_bytes(&key.to_bytes());
    assert_eq!(again.reader_id(), key.reader_id());
    assert_ne!(ReaderKey::generate().reader_id(), key.reader_id());
    let printed = format!("{key:?}");
    assert!(printed.contains("redacted"));
    let seed: String = key
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert!(!printed.contains(&seed));
    let id = key.reader_id();
    assert_eq!(ReaderId::from_bytes(id.to_bytes()), id);
    assert!(format!("{id:?}").starts_with("ReaderId("));
    assert!(matches!(
        ComputerKind::this(),
        ComputerKind::Linux | ComputerKind::MacOs | ComputerKind::Windows | ComputerKind::Other
    ));
}
