//! What Home's foot and the tray show (ADR 0033 rules 6 to 8 and 10a).

use semver::Version;

use super::look::DAY_MS;
use super::notice::plain_version;
use super::state::VersionLook;

/// Off, a version this many days old says so.
pub const OLD_AFTER_DAYS: u64 = 90;

/// One line, or none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeLine {
    Nothing,
    /// "OurStick 0.3.0 is out · Get it".
    Newer {
        version: String,
    },
    /// "0.3.0 fixes a security problem in this version · Get it", in amber.
    Security {
        version: String,
    },
    /// The release key changed: the new version is installed by hand.
    ReleaseKeyChanged,
    /// Looking is off and this version is old: "This version is from
    /// March 2027. A newer one may fix security problems · ourstick.com".
    Old {
        year: u32,
        month: u32,
    },
}

/// The day this version was released, built in by the release step
/// (`release-day.txt`); none in a debug build.
pub fn release_day() -> Option<u64> {
    if cfg!(debug_assertions) {
        None
    } else {
        day_number(include_str!("../../release-day.txt").trim())
    }
}

/// The line for this version (`current`, with the built-in release key's
/// id `release_key_id`) from what was kept.
pub fn notice_line(
    look: &VersionLook,
    current: &Version,
    release_key_id: &str,
    release_day: Option<u64>,
    now_ms: u64,
) -> NoticeLine {
    if look
        .new_release_key
        .as_deref()
        .is_some_and(|new| new != release_key_id)
    {
        return NoticeLine::ReleaseKeyChanged;
    }
    let known = |text: &Option<String>| text.as_deref().and_then(|t| plain_version(t).ok());
    let newest = known(&look.newest);
    if let Some(security) = known(&look.security) {
        if *current < security {
            let version = newest
                .filter(|newest| *newest >= security)
                .unwrap_or(security);
            return NoticeLine::Security {
                version: version.to_string(),
            };
        }
    }
    if look.looking {
        return match newest {
            Some(newest) if newest > *current => NoticeLine::Newer {
                version: newest.to_string(),
            },
            _ => NoticeLine::Nothing,
        };
    }
    match release_day {
        Some(day) if now_ms / DAY_MS > day + OLD_AFTER_DAYS => {
            let (year, month, _) = civil_from_days(day);
            NoticeLine::Old { year, month }
        }
        _ => NoticeLine::Nothing,
    }
}

/// True when the window should tell of the security line once (rule 8).
pub fn security_to_tell(look: &VersionLook, line: &NoticeLine) -> bool {
    matches!(line, NoticeLine::Security { .. }) && look.security_told != look.security
}

/// Days since 1970-01-01 of a `YYYY-MM-DD`.
pub fn day_number(text: &str) -> Option<u64> {
    let mut parts = text.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let (year, month, day): (i64, i64, i64) =
        (year.parse().ok()?, month.parse().ok()?, day.parse().ok()?);
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let year_of_era = shifted - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = u64::try_from(era * 146_097 + day_of_era - 719_468).ok()?;
    // A day that does not exist (31 April) comes back as another.
    let back = civil_from_days(days);
    (back == (year as u32, month as u32, day as u32)).then_some(days)
}

/// The `(year, month, day)` of days since 1970-01-01.
pub fn civil_from_days(days: u64) -> (u32, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year as u32, month as u32, day as u32)
}

#[cfg(test)]
#[path = "line_tests.rs"]
mod tests;
