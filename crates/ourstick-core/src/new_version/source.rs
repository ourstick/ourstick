//! Reading `version.json` (ADR 0033 rule 2), behind a trait so that tests
//! read from memory (CLAUDE.md rule 2).

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use super::state::LookFailure;

/// The file is small; more than this is not it.
pub const MOST_BYTES: usize = 64 * 1024;
/// How long a look may take, all of it.
pub const LOOK_TIMEOUT: Duration = Duration::from_secs(15);
/// Where the notice is.
pub const NOTICE_URL: &str = "https://ourstick.com/version.json";

/// Why a read failed. Never holds what was read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
    #[error("no connection: {0}")]
    NoConnection(String),
    #[error("answered with status {0}")]
    Refused(u16),
    #[error("more than {MOST_BYTES} bytes")]
    TooBig,
}

impl ReadError {
    pub fn failure(&self) -> LookFailure {
        match self {
            Self::NoConnection(_) => LookFailure::NoConnection,
            Self::Refused(_) => LookFailure::Refused,
            Self::TooBig => LookFailure::NotTheFile,
        }
    }
}

/// Reads the notice file.
pub trait NoticeSource: Send + Sync {
    fn read(&self) -> impl Future<Output = Result<Vec<u8>, ReadError>> + Send;
}

/// Over HTTPS: nothing about the computer is sent (no cookie, no query, no
/// `User-Agent`, no `Referer`, no cache headers), no redirect is followed,
/// at most [`MOST_BYTES`] are read, within [`LOOK_TIMEOUT`]. The proxy the
/// environment names is used, as for the relays (ADR 0006).
#[derive(Debug, Clone)]
pub struct HttpSource {
    client: reqwest::Client,
    url: String,
}

impl HttpSource {
    /// The real file. A debug build may read another (rule 13), over plain
    /// HTTP too; a release build only over HTTPS.
    pub fn new(url: &str) -> Result<Self, ReadError> {
        let tls = iroh::tls::CaTlsConfig::embedded()
            .client_config(iroh::tls::default_provider())
            .map_err(|error| ReadError::NoConnection(error.to_string()))?;
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .https_only(!cfg!(debug_assertions))
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .timeout(LOOK_TIMEOUT)
            .build()
            .map_err(|error| ReadError::NoConnection(error.to_string()))?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }
}

impl NoticeSource for HttpSource {
    async fn read(&self) -> Result<Vec<u8>, ReadError> {
        let no_connection = |error: reqwest::Error| ReadError::NoConnection(error.to_string());
        let mut response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(no_connection)?;
        let status = response.status();
        if status != reqwest::StatusCode::OK {
            return Err(ReadError::Refused(status.as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MOST_BYTES as u64)
        {
            return Err(ReadError::TooBig);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(no_connection)? {
            if bytes.len() + chunk.len() > MOST_BYTES {
                return Err(ReadError::TooBig);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

/// In memory, for tests: answers in turn, and counts the reads.
#[derive(Debug, Default)]
pub struct MemorySource {
    answers: Mutex<VecDeque<Result<Vec<u8>, ReadError>>>,
    reads: AtomicUsize,
}

impl MemorySource {
    pub fn answer(&self, answer: Result<Vec<u8>, ReadError>) {
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(answer);
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl NoticeSource for MemorySource {
    async fn read(&self) -> Result<Vec<u8>, ReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.answers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or_else(|| Err(ReadError::NoConnection("no answer set".into())))
    }
}

#[cfg(test)]
#[path = "source_tests.rs"]
mod tests;
