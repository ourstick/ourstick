//! Telling users of a new version (ADR 0033, step 1): once a day the
//! desktop app reads a signed `version.json` from ourstick.com, sending
//! nothing about the computer, and shows one line when a newer version, or
//! a security fix, is out. The core decides; the app only shows.

mod keys;
mod line;
mod look;
mod notice;
mod signature;
mod source;
mod state;
#[cfg(test)]
mod test_keys;

pub use keys::{NoticeKey, NoticeKeys};
pub use line::{
    civil_from_days, day_number, notice_line, release_day, security_to_tell, NoticeLine,
    OLD_AFTER_DAYS,
};
pub use look::{
    is_due, keep_outcome, look_if_due, read_outcome, set_looking, told_looking, told_security,
    DAY_MS, HOUR_MS,
};
pub use notice::{plain_version, read_notice, Notice, NoticeError, System};
pub use signature::{SignatureError, Signer};
pub use source::{
    HttpSource, MemorySource, NoticeSource, ReadError, LOOK_TIMEOUT, MOST_BYTES, NOTICE_URL,
};
pub use state::{LookFailure, MemoryNoticeStore, NoticeStore, VersionLook};

/// This build's version, as a version.
pub fn this_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version is semver")
}
