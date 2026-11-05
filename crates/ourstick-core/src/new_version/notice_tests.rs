use super::super::test_keys::{test_keys, version_json};
use super::*;

const GOOD: &str = r#"{"serial":3,"made":"2026-11-20",
 "windows":{"newest":"0.3.0","security":"0.2.2"},
 "macos":{"newest":"0.3.1","security":"0.3.1"},
 "linux":{"newest":"0.3.0","security":"0.1.0"},
 "later":"a field a later version adds"}"#;

fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn a_good_notice_is_read_for_each_system() {
    let (release, _, keys) = test_keys();
    let file = version_json(&release, GOOD);
    let Notice::Versions {
        serial,
        key_id,
        newest,
        security,
    } = read_notice(&file, &keys, System::Macos).unwrap()
    else {
        panic!("versions");
    };
    assert_eq!(serial, 3);
    assert_eq!(key_id, keys.release.id_hex());
    assert_eq!((newest, security), (version("0.3.1"), version("0.3.1")));
    let windows = read_notice(&file, &keys, System::Windows).unwrap();
    assert!(matches!(windows, Notice::Versions { security, .. } if security == version("0.2.2")));
}

#[test]
fn what_is_not_the_file_is_refused() {
    let (_, _, keys) = test_keys();
    for bytes in [
        &b"<html><body>Log in to the hotel's network</body></html>"[..],
        b"",
        b"{}",
        b"{\"notice\": 3, \"signature\": \"x\"}",
    ] {
        assert_eq!(
            read_notice(bytes, &keys, System::Linux),
            Err(NoticeError::NotTheFile)
        );
    }
}

#[test]
fn a_notice_of_another_key_is_refused_before_it_is_parsed() {
    let (_, _, keys) = test_keys();
    let stranger = super::super::test_keys::TestKey::new(7);
    let file = version_json(&stranger, "not even json");
    assert_eq!(
        read_notice(&file, &keys, System::Linux),
        Err(NoticeError::Signature(SignatureError::OtherKey))
    );
}

#[test]
fn a_signed_notice_that_does_not_hold_is_ignored_whole() {
    let (release, _, keys) = test_keys();
    for notice in [
        "not json",
        r#"{"serial":1,"made":"2026-11-20"}"#,
        r#"{"serial":1,"made":"2026-11-20","linux":{"newest":"0.3","security":"0.3"}}"#,
        r#"{"serial":1,"made":"2026-11-20","linux":{"newest":"0.3.0-beta.1","security":"0.2.0"}}"#,
        r#"{"serial":1,"made":"2026-11-20","linux":{"newest":"v0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":1,"made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.4.0"}}"#,
        r#"{"serial":-1,"made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":1.5,"made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":"1","made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":9223372036854775808,"made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":1,"made":"tomorrow","linux":{"newest":"0.3.0","security":"0.2.0"}}"#,
        r#"{"serial":1,"made":"2026-11-20","linux":{"newest":"0.3.0","security":"0.2.0"},"new_release_key":"0011223344556677"}"#,
    ] {
        let file = version_json(&release, notice);
        assert!(
            matches!(
                read_notice(&file, &keys, System::Linux),
                Err(NoticeError::Unusable(_))
            ),
            "{notice}"
        );
    }
}

#[test]
fn the_recovery_key_only_says_the_release_key_changed() {
    let (_, recovery, keys) = test_keys();
    let file = version_json(
        &recovery,
        r#"{"serial":1,"made":"2027-01-02","new_release_key":"aabbccddeeff0011"}"#,
    );
    assert_eq!(
        read_notice(&file, &keys, System::Windows),
        Ok(Notice::ReleaseKeyChanged {
            serial: 1,
            key_id: keys.recovery.id_hex(),
            new_release_key: "AABBCCDDEEFF0011".into(),
        })
    );
    // Versions from the recovery key are not taken.
    let versions = version_json(&recovery, GOOD);
    assert!(matches!(
        read_notice(&versions, &keys, System::Windows),
        Err(NoticeError::Unusable(_))
    ));
}

#[test]
fn versions_are_compared_as_versions() {
    assert!(plain_version("0.10.0").unwrap() > plain_version("0.9.0").unwrap());
    assert!(plain_version("1.0.0").unwrap() > plain_version("0.99.99").unwrap());
    for bad in ["0.3", "00.3.0", "0.3.0+build", "0.3.0-rc.1", " 0.3.0", ""] {
        assert!(plain_version(bad).is_err(), "{bad}");
    }
}

/// Made by `tools/release/version-json.sh 0.3.0 0.3.0` with the throwaway
/// key of `signature_tests`, so that what the release makes is what the app
/// reads.
#[test]
fn a_file_made_by_the_release_script_is_read() {
    let keys = NoticeKeys {
        release: super::super::keys::NoticeKey::parse(
            "RWQJQpHcVK9K5fW1trUDReOj8I82vl+ktK4saboi4AAunz3ScMaGkD72",
        )
        .unwrap(),
        recovery: super::super::test_keys::TestKey::new(9).public(),
    };
    let file = include_bytes!("test-version.json");
    for system in [System::Windows, System::Macos, System::Linux] {
        assert_eq!(
            read_notice(file, &keys, system),
            Ok(Notice::Versions {
                serial: 2,
                key_id: "E54AAF54DC914209".into(),
                newest: version("0.3.0"),
                security: version("0.3.0"),
            })
        );
    }
}
