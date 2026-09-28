//! Running blocking work (file I/O, SQLite, hashing) from async code without
//! blocking the runtime (CLAUDE.md rule 7).

/// Runs `work` on tokio's blocking thread pool. A panic inside `work` is
/// resumed in the caller, as if `work` had run inline.
pub(crate) async fn run_blocking<T, F>(work: F) -> T
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => value,
        Err(error) if error.is_panic() => std::panic::resume_unwind(error.into_panic()),
        Err(error) => panic!("blocking task was cancelled: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_the_value() {
        assert_eq!(run_blocking(|| 2 + 2).await, 4);
    }

    #[tokio::test]
    #[should_panic(expected = "inside")]
    async fn resumes_panics() {
        run_blocking(|| panic!("inside")).await
    }
}
