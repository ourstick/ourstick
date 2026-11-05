//! What the app keeps of its looks for a new version (ADR 0033 rules 5 and
//! 10), in SQLite like all its state (`version_look`, `notice_serials`).

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::store::StoreError;

/// Why the last look found nothing it could take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookFailure {
    /// No answer from ourstick.com (no network, a proxy, a timeout).
    NoConnection,
    /// It answered, but not with the file (404, 500, a redirect).
    Refused,
    /// A file that is not a good notice: a captive portal's page, a bad
    /// signature, a notice that does not hold.
    NotTheFile,
}

impl LookFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoConnection => "no-connection",
            Self::Refused => "refused",
            Self::NotTheFile => "not-the-file",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::NoConnection, Self::Refused, Self::NotTheFile]
            .into_iter()
            .find(|failure| failure.as_str() == text)
    }
}

/// The one row of `version_look`, and the serials per key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionLook {
    /// "Look for new versions" (on unless turned off).
    pub looking: bool,
    /// The window said once that the app now looks (rule 11).
    pub looking_told: bool,
    /// When the next look is due; 0 for at once.
    pub next_look_ms: u64,
    /// The last look that found a good notice.
    pub last_good_ms: Option<u64>,
    /// Since when every look failed, and how the last one did.
    pub failing_since_ms: Option<u64>,
    pub failure: Option<LookFailure>,
    /// The newest version a notice named, forgotten when looking is off.
    pub newest: Option<String>,
    /// The oldest safe version a notice named, kept when looking is off.
    pub security: Option<String>,
    /// The id of a new release key the recovery key announced.
    pub new_release_key: Option<String>,
    /// The `security` the window has told of once.
    pub security_told: Option<String>,
    /// The highest serial taken, per id of the key that signed it.
    pub serials: BTreeMap<String, u64>,
}

impl Default for VersionLook {
    fn default() -> Self {
        Self {
            looking: true,
            looking_told: false,
            next_look_ms: 0,
            last_good_ms: None,
            failing_since_ms: None,
            failure: None,
            newest: None,
            security: None,
            new_release_key: None,
            security_told: None,
            serials: BTreeMap::new(),
        }
    }
}

/// Where [`VersionLook`] is kept.
pub trait NoticeStore: Send + Sync {
    fn version_look(&self) -> Result<VersionLook, StoreError>;
    fn set_version_look(&self, look: &VersionLook) -> Result<(), StoreError>;
}

/// In memory, for tests.
#[derive(Debug, Default)]
pub struct MemoryNoticeStore {
    look: Mutex<VersionLook>,
}

impl NoticeStore for MemoryNoticeStore {
    fn version_look(&self) -> Result<VersionLook, StoreError> {
        Ok(self.look.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn set_version_look(&self, look: &VersionLook) -> Result<(), StoreError> {
        *self.look.lock().unwrap_or_else(|e| e.into_inner()) = look.clone();
        Ok(())
    }
}
