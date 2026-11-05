use super::*;

const KEY: &str = "2DDEFEB70EAC0BBB";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn look(newest: Option<&str>, security: Option<&str>) -> VersionLook {
    VersionLook {
        newest: newest.map(String::from),
        security: security.map(String::from),
        ..VersionLook::default()
    }
}

fn line(look: &VersionLook, current: &str) -> NoticeLine {
    notice_line(look, &v(current), KEY, None, 0)
}

#[test]
fn nothing_when_this_is_the_newest_or_newer() {
    let known = look(Some("0.3.0"), Some("0.2.0"));
    assert_eq!(line(&known, "0.3.0"), NoticeLine::Nothing);
    // A test build, or the minutes after a release.
    assert_eq!(line(&known, "0.4.0"), NoticeLine::Nothing);
    assert_eq!(line(&VersionLook::default(), "0.3.0"), NoticeLine::Nothing);
}

#[test]
fn a_newer_version_is_compared_as_a_version() {
    let known = look(Some("0.10.0"), Some("0.1.0"));
    assert_eq!(
        line(&known, "0.9.0"),
        NoticeLine::Newer {
            version: "0.10.0".into()
        }
    );
}

#[test]
fn older_than_security_shows_the_security_line_equal_does_not() {
    let known = look(Some("0.3.1"), Some("0.3.0"));
    assert_eq!(
        line(&known, "0.2.9"),
        NoticeLine::Security {
            version: "0.3.1".into()
        }
    );
    assert_eq!(
        line(&known, "0.3.0"),
        NoticeLine::Newer {
            version: "0.3.1".into()
        }
    );
}

#[test]
fn off_forgets_newer_but_keeps_security() {
    let mut known = look(None, Some("0.3.0"));
    known.looking = false;
    assert_eq!(
        line(&known, "0.2.0"),
        NoticeLine::Security {
            version: "0.3.0".into()
        }
    );
    assert_eq!(line(&known, "0.3.0"), NoticeLine::Nothing);
}

#[test]
fn a_new_release_key_shows_until_the_app_has_it() {
    let mut known = look(Some("0.3.0"), Some("0.3.0"));
    known.new_release_key = Some("AABBCCDDEEFF0011".into());
    assert_eq!(line(&known, "0.2.0"), NoticeLine::ReleaseKeyChanged);
    assert_eq!(
        notice_line(&known, &v("0.4.0"), "AABBCCDDEEFF0011", None, 0),
        NoticeLine::Nothing
    );
}

#[test]
fn off_an_old_version_says_so_after_90_days() {
    let released = day_number("2027-03-10").unwrap();
    let mut off = VersionLook {
        looking: false,
        ..VersionLook::default()
    };
    let at = |days: u64| (released + days) * DAY_MS + 5;
    let old = NoticeLine::Old {
        year: 2027,
        month: 3,
    };
    assert_eq!(
        notice_line(&off, &v("0.3.0"), KEY, Some(released), at(91)),
        old
    );
    assert_eq!(
        notice_line(&off, &v("0.3.0"), KEY, Some(released), at(89)),
        NoticeLine::Nothing
    );
    // No release day (a debug build), or a clock before it: nothing.
    assert_eq!(
        notice_line(&off, &v("0.3.0"), KEY, None, at(400)),
        NoticeLine::Nothing
    );
    assert_eq!(
        notice_line(&off, &v("0.3.0"), KEY, Some(released), 5),
        NoticeLine::Nothing
    );
    // On, never: the notice says better.
    off.looking = true;
    assert_eq!(
        notice_line(&off, &v("0.3.0"), KEY, Some(released), at(400)),
        NoticeLine::Nothing
    );
}

#[test]
fn the_security_line_is_told_once() {
    let mut known = look(Some("0.3.0"), Some("0.3.0"));
    let shown = line(&known, "0.2.0");
    assert!(security_to_tell(&known, &shown));
    known.security_told = known.security.clone();
    assert!(!security_to_tell(&known, &shown));
    // A later security fix is told again.
    known.security = Some("0.3.1".into());
    assert!(security_to_tell(&known, &line(&known, "0.3.0")));
    assert!(!security_to_tell(&known, &NoticeLine::Nothing));
}

#[test]
fn days_are_counted_from_1970() {
    assert_eq!(day_number("1970-01-01"), Some(0));
    assert_eq!(day_number("2026-11-05"), Some(20_762));
    assert_eq!(civil_from_days(20_762), (2026, 11, 5));
    assert_eq!(
        day_number("2028-02-29").map(civil_from_days),
        Some((2028, 2, 29))
    );
    for bad in [
        "2026-04-31",
        "2027-02-29",
        "2026-13-01",
        "2026-11",
        "x",
        "1969-12-31",
        "",
    ] {
        assert_eq!(day_number(bad), None, "{bad}");
    }
}

#[test]
fn the_built_in_release_day_is_a_day() {
    assert!(day_number(include_str!("../../release-day.txt").trim()).is_some());
}
