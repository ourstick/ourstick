use super::*;
use rand::{Rng, SeedableRng};

fn owner() -> StickSigningKey {
    StickSigningKey::from_bytes(&[7; 32])
}

/// A ticket hint below every record used in these tests.
const NO_FLOOR: ManifestHint = ManifestHint {
    seq: 0,
    hash: [0; 32],
};

#[test]
fn signed_record_roundtrips_and_verifies() {
    let signed = owner().sign_record(5, [9; 32]);
    let bytes = signed.to_bytes();
    assert_eq!(bytes.len(), SIGNED_RECORD_LEN);

    let parsed = SignedManifestRecord::from_bytes(&bytes).unwrap();
    assert_eq!(parsed, signed);
    let record = parsed.verify(&owner().stick_id(), &NO_FLOOR, None).unwrap();
    assert_eq!(
        record,
        ManifestRecord {
            stick_id: owner().stick_id(),
            seq: 5,
            manifest_hash: [9; 32],
        }
    );
}

#[test]
fn record_layout_is_fixed() {
    let bytes = owner()
        .sign_record(0x0102_0304_0506_0708, [9; 32])
        .to_bytes();
    assert_eq!(bytes[0], RECORD_FORMAT_VERSION);
    assert_eq!(&bytes[1..33], &owner().stick_id().to_bytes());
    assert_eq!(&bytes[33..41], &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(&bytes[41..73], &[9; 32]);
}

#[test]
fn verify_rejects_older_or_equal_seq() {
    let signed = owner().sign_record(5, [9; 32]);
    let stick_id = owner().stick_id();
    assert!(signed.verify(&stick_id, &NO_FLOOR, Some(4)).is_ok());
    for last_seen in [5, 6, u64::MAX] {
        assert_eq!(
            signed.verify(&stick_id, &NO_FLOOR, Some(last_seen)),
            Err(RecordError::NotNewer { seq: 5, last_seen })
        );
    }
}

#[test]
fn verify_rejects_record_older_than_ticket() {
    let hint = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    let older = owner().sign_record(4, [8; 32]);
    assert_eq!(
        older.verify(&owner().stick_id(), &hint, None),
        Err(RecordError::OlderThanTicket {
            seq: 4,
            ticket_seq: 5
        })
    );
}

#[test]
fn verify_requires_ticket_hash_for_ticket_seq() {
    let hint = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    let matching = owner().sign_record(5, [9; 32]);
    assert!(matching.verify(&owner().stick_id(), &hint, None).is_ok());

    let conflicting = owner().sign_record(5, [8; 32]);
    assert_eq!(
        conflicting.verify(&owner().stick_id(), &hint, None),
        Err(RecordError::ConflictsWithTicket { seq: 5 })
    );
}

#[test]
fn verify_accepts_newer_than_ticket_with_any_hash() {
    let hint = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    let newer = owner().sign_record(6, [8; 32]);
    assert!(newer.verify(&owner().stick_id(), &hint, None).is_ok());
}

#[test]
fn verify_applies_ticket_floor_and_last_seen_together() {
    let hint = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    // Last seen is below the ticket (e.g. from an older ticket): the ticket wins.
    let older = owner().sign_record(4, [8; 32]);
    assert!(matches!(
        older.verify(&owner().stick_id(), &hint, Some(3)),
        Err(RecordError::OlderThanTicket { .. })
    ));
    // Last seen is above the ticket: last seen wins.
    let seven = owner().sign_record(7, [8; 32]);
    assert_eq!(
        seven.verify(&owner().stick_id(), &hint, Some(7)),
        Err(RecordError::NotNewer {
            seq: 7,
            last_seen: 7
        })
    );
    assert!(seven.verify(&owner().stick_id(), &hint, Some(6)).is_ok());
}

#[test]
fn verify_checks_signature_before_seq() {
    // A forged record must be reported as forged, not as stale.
    let forged = StickSigningKey::from_bytes(&[8; 32]).sign_record(1, [0; 32]);
    let mut bytes = forged.to_bytes();
    bytes[1..33].copy_from_slice(&owner().stick_id().to_bytes());
    let parsed = SignedManifestRecord::from_bytes(&bytes).unwrap();
    let hint = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    assert_eq!(
        parsed.verify(&owner().stick_id(), &hint, Some(10)),
        Err(RecordError::BadSignature)
    );
}

#[test]
fn verify_rejects_record_of_another_stick() {
    let signed = owner().sign_record(1, [9; 32]);
    let other = StickSigningKey::from_bytes(&[8; 32]).stick_id();
    assert_eq!(
        signed.verify(&other, &NO_FLOOR, None),
        Err(RecordError::WrongStick)
    );
}

#[test]
fn verify_rejects_any_tampered_field() {
    let bytes = owner().sign_record(1, [9; 32]).to_bytes();
    // seq, manifest hash, and signature bytes.
    for index in [33, 40, 41, 72, 73, 136] {
        let mut tampered = bytes;
        tampered[index] ^= 1;
        let parsed = SignedManifestRecord::from_bytes(&tampered).unwrap();
        assert_eq!(
            parsed.verify(&owner().stick_id(), &NO_FLOOR, None),
            Err(RecordError::BadSignature),
            "byte {index}"
        );
    }
}

#[test]
fn verify_rejects_record_signed_by_someone_else_for_this_stick() {
    // An attacker copies the StickId into a record signed with their own key.
    let forged = StickSigningKey::from_bytes(&[8; 32]).sign_record(99, [0; 32]);
    let mut bytes = forged.to_bytes();
    bytes[1..33].copy_from_slice(&owner().stick_id().to_bytes());
    let parsed = SignedManifestRecord::from_bytes(&bytes).unwrap();
    assert_eq!(
        parsed.verify(&owner().stick_id(), &NO_FLOOR, None),
        Err(RecordError::BadSignature)
    );
}

#[test]
fn from_bytes_rejects_bad_layout() {
    let bytes = owner().sign_record(1, [9; 32]).to_bytes();
    assert_eq!(
        SignedManifestRecord::from_bytes(&bytes[..136]),
        Err(RecordError::WrongLength(136))
    );
    assert_eq!(
        SignedManifestRecord::from_bytes(&[bytes.as_slice(), &[0]].concat()),
        Err(RecordError::WrongLength(138))
    );
    let mut wrong_version = bytes;
    wrong_version[0] = 2;
    assert_eq!(
        SignedManifestRecord::from_bytes(&wrong_version),
        Err(RecordError::UnsupportedVersion(2))
    );
    let mut bad_key = bytes;
    bad_key[1..33].copy_from_slice(&[0; 32]);
    bad_key[1] = 2; // y = 2 is not on the curve
    assert_eq!(
        SignedManifestRecord::from_bytes(&bad_key),
        Err(RecordError::InvalidStickId)
    );
}

#[test]
fn from_bytes_never_panics_on_random_input() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x005E_C04D);
    for _ in 0..2_000 {
        let length = if rng.gen_bool(0.5) {
            SIGNED_RECORD_LEN
        } else {
            rng.gen_range(0..200)
        };
        let mut bytes: Vec<u8> = (0..length).map(|_| rng.gen()).collect();
        if let Some(first) = bytes.first_mut() {
            *first = RECORD_FORMAT_VERSION;
        }
        if let Ok(parsed) = SignedManifestRecord::from_bytes(&bytes) {
            let _ = parsed.verify(&owner().stick_id(), &NO_FLOOR, None);
        }
    }
}

#[test]
fn next_seq_follows_the_clock_but_always_increases() {
    // Clock ahead of the stored value: the clock wins.
    assert_eq!(next_seq(5, 1_700_000_000_000), Some(1_700_000_000_000));
    // Clock behind or equal (stalled, set back): last + 1 wins.
    assert_eq!(
        next_seq(1_700_000_000_000, 1_600_000_000_000),
        Some(1_700_000_000_001)
    );
    assert_eq!(
        next_seq(1_700_000_000_000, 1_700_000_000_000),
        Some(1_700_000_000_001)
    );
    // Nothing stored yet.
    assert_eq!(next_seq(0, 0), Some(1));
    // Exhausted.
    assert_eq!(next_seq(u64::MAX, 0), None);
}

#[test]
fn unverified_record_exposes_what_was_signed() {
    let signed = owner().sign_record(9, [1; 32]);
    assert_eq!(signed.unverified_record().seq, 9);
    assert_eq!(signed.unverified_record().stick_id, owner().stick_id());
}

#[test]
fn check_distinguishes_newer_unchanged_older_and_conflict() {
    let hint = NO_FLOOR;
    let current = ManifestHint {
        seq: 5,
        hash: [9; 32],
    };
    let stick_id = owner().stick_id();

    let first = owner().sign_record(5, [9; 32]);
    assert!(matches!(
        first.check(&stick_id, &hint, None),
        Ok(RecordCheck::Newer(record)) if record.seq == 5
    ));
    assert!(matches!(
        first.check(&stick_id, &hint, Some(&current)),
        Ok(RecordCheck::Unchanged(record)) if record.seq == 5
    ));
    assert!(matches!(
        owner().sign_record(6, [1; 32]).check(&stick_id, &hint, Some(&current)),
        Ok(RecordCheck::Newer(record)) if record.seq == 6
    ));
    assert_eq!(
        owner()
            .sign_record(4, [9; 32])
            .check(&stick_id, &hint, Some(&current)),
        Err(RecordError::NotNewer {
            seq: 4,
            last_seen: 5
        })
    );
    assert_eq!(
        owner()
            .sign_record(5, [1; 32])
            .check(&stick_id, &hint, Some(&current)),
        Err(RecordError::ConflictsWithCurrent { seq: 5 })
    );
}

#[test]
fn check_still_enforces_ticket_floor_and_signature() {
    let hint = ManifestHint {
        seq: 10,
        hash: [9; 32],
    };
    let stick_id = owner().stick_id();
    assert!(matches!(
        owner()
            .sign_record(9, [9; 32])
            .check(&stick_id, &hint, None),
        Err(RecordError::OlderThanTicket { .. })
    ));
    let forged = StickSigningKey::from_bytes(&[8; 32]).sign_record(20, [0; 32]);
    let mut bytes = forged.to_bytes();
    bytes[1..33].copy_from_slice(&stick_id.to_bytes());
    let parsed = SignedManifestRecord::from_bytes(&bytes).unwrap();
    assert_eq!(
        parsed.check(&stick_id, &hint, None),
        Err(RecordError::BadSignature)
    );
}
