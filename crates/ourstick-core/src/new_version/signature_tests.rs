use super::super::test_keys::{test_keys, TestKey};
use super::*;

const NOTICE: &[u8] = br#"{"serial":1,"made":"2026-11-05"}"#;

#[test]
fn a_good_signature_says_which_key() {
    let (release, recovery, keys) = test_keys();
    let by_release = release.sign(NOTICE);
    let by_recovery = recovery.sign(NOTICE);
    assert_eq!(
        check(NOTICE, &by_release, &keys.release, &keys.recovery),
        Ok(Signer::Release)
    );
    assert_eq!(
        check(NOTICE, &by_recovery, &keys.release, &keys.recovery),
        Ok(Signer::Recovery)
    );
}

/// Made by the real `minisign -S -l` with a throwaway key (no password), so
/// that this check is the tool's, not only our own idea of it.
#[test]
fn a_signature_made_by_minisign_itself_is_good() {
    let key = NoticeKey::parse("RWQJQpHcVK9K5fW1trUDReOj8I82vl+ktK4saboi4AAunz3ScMaGkD72")
        .expect("a key");
    let signature = "untrusted comment: signature from minisign secret key
RWQJQpHcVK9K5TGdoPJ/WwDqWR+NAWOYyMcyiGAVImXq/w0ZfxeAdAFoIAioZyR+YIaA+CC5flR5BpcFMC6fjQTKIxNtDBcZXwg=
trusted comment: OurStick test notice
ny7GvhwkDy99X+/c+Kww2Y72HfxSFxuvo8h5b0uZ60hvbDOg5/0xX6zkg3h/sRxvanFcDLNPyHde04X11oynDg==
";
    let other = TestKey::new(9).public();
    assert_eq!(check(NOTICE, signature, &key, &other), Ok(Signer::Release));
    assert_eq!(key.id_hex(), "E54AAF54DC914209");
    // minisign's default, prehashed (`ED`), from the same key: refused.
    let prehashed = "untrusted comment: signature from minisign secret key
RUQJQpHcVK9K5RUM959ACX+xBJQudobEXI2Kiq7VXe3aGTyOjpKxkoM+b60HiQP1bb+oX8wvqCUzN4PzsCUNWlewzaBcnyUlMgk=
trusted comment: prehashed
0tPQB8sJlztONsq3ql1/D+ayptpeiIAWYPXrSv318Yv0rC+t5LS52WaB6pCe7bT6ACnSSdJS33P+h2Cg7+ElBA==
";
    assert_eq!(
        check(NOTICE, prehashed, &key, &other),
        Err(SignatureError::NotLegacy)
    );
}

#[test]
fn another_key_is_refused() {
    let (_, _, keys) = test_keys();
    let stranger = TestKey::new(7).sign(NOTICE);
    assert_eq!(
        check(NOTICE, &stranger, &keys.release, &keys.recovery),
        Err(SignatureError::OtherKey)
    );
}

#[test]
fn a_changed_notice_under_an_old_signature_is_refused() {
    let (release, _, keys) = test_keys();
    let signature = release.sign(NOTICE);
    let changed = br#"{"serial":2,"made":"2026-11-05"}"#;
    assert_eq!(
        check(changed, &signature, &keys.release, &keys.recovery),
        Err(SignatureError::Mismatch)
    );
}

#[test]
fn a_changed_trusted_comment_is_refused() {
    let (release, _, keys) = test_keys();
    let signature = release.sign(NOTICE).replace(
        "trusted comment: OurStick test",
        "trusted comment: OurStick tost",
    );
    assert_eq!(
        check(NOTICE, &signature, &keys.release, &keys.recovery),
        Err(SignatureError::Mismatch)
    );
}

#[test]
fn a_prehashed_signature_is_refused() {
    let (release, _, keys) = test_keys();
    let signature = release.sign_as(b"ED", NOTICE, "x");
    assert_eq!(
        check(NOTICE, &signature, &keys.release, &keys.recovery),
        Err(SignatureError::NotLegacy)
    );
}

#[test]
fn a_truncated_or_odd_signature_is_refused() {
    let (release, _, keys) = test_keys();
    let good = release.sign(NOTICE);
    let lines: Vec<&str> = good.lines().collect();
    let truncated_line = format!(
        "{}\n{}\n{}\n{}\n",
        lines[0],
        &lines[1][..lines[1].len() - 8],
        lines[2],
        lines[3]
    );
    for bad in [
        String::new(),
        lines[..3].join("\n"),
        truncated_line,
        good.replace("untrusted comment: ", "comment: "),
        good.replace("trusted comment: OurStick", "OurStick"),
        format!("{good}one line more\n"),
        "untrusted comment: x\n!!!\ntrusted comment: y\n!!!\n".to_string(),
    ] {
        assert!(
            check(NOTICE, &bad, &keys.release, &keys.recovery).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn windows_line_ends_are_taken() {
    let (release, _, keys) = test_keys();
    let signature = release.sign(NOTICE).replace('\n', "\r\n");
    assert_eq!(
        check(NOTICE, &signature, &keys.release, &keys.recovery),
        Ok(Signer::Release)
    );
}
