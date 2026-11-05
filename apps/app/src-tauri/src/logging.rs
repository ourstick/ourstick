//! The app's log.

/// The log filter: `RUST_LOG`, with `tao` never below `debug`, since at
/// `trace` it logs the addresses the app was opened with, and one may hold
/// a key (ADR 0030).
pub fn log_filter() -> tracing_subscriber::EnvFilter {
    let filter = tracing_subscriber::EnvFilter::from_default_env();
    // Without RUST_LOG only errors are logged: nothing to cap, and a cap
    // would let tao's debug lines through.
    if std::env::var_os("RUST_LOG").is_none() {
        return filter;
    }
    match "tao=debug".parse() {
        Ok(cap) => filter.add_directive(cap),
        Err(_) => filter,
    }
}
