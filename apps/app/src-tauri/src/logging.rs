//! The app's log.

use tracing_subscriber::filter::LevelFilter;

/// The log filter: `RUST_LOG`, with `tao` never below `debug`, since at
/// `trace` it logs the addresses the app was opened with, and one may hold
/// a key (ADR 0030).
pub fn log_filter() -> tracing_subscriber::EnvFilter {
    // Without RUST_LOG only errors are logged: nothing to cap, and a cap
    // would let tao's debug lines through.
    let Some(rust_log) = std::env::var_os("RUST_LOG") else {
        return tracing_subscriber::EnvFilter::from_default_env();
    };
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(LevelFilter::ERROR.into())
        .parse_lossy(without_tao(&rust_log.to_string_lossy()));
    match "tao=debug".parse() {
        Ok(cap) => filter.add_directive(cap),
        Err(_) => filter,
    }
}

/// `directives` without any for `tao` or a module of it: a more specific
/// one (`tao::platform_impl=trace`) would win over the cap. A filter takes a
/// target as a prefix of the module path, so any target longer than `tao`
/// that can match one of tao's starts with `tao:` (`tao:`, `tao::`).
fn without_tao(directives: &str) -> String {
    directives
        .split(',')
        .filter(|directive| {
            let target = directive.split(['=', '[']).next().unwrap_or("").trim();
            target != "tao" && !target.starts_with("tao:")
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directives_for_tao_are_dropped_and_others_kept() {
        assert_eq!(
            without_tao("ourstick_core=debug,tao::platform_impl=trace,tao,iroh=info"),
            "ourstick_core=debug,iroh=info"
        );
        assert_eq!(without_tao("trace"), "trace");
        assert_eq!(without_tao("taomaker=trace"), "taomaker=trace");
        assert_eq!(without_tao("tao[span]=trace"), "");
        assert_eq!(without_tao("tao:=trace,iroh=info"), "iroh=info");
        assert_eq!(without_tao("tao:"), "");
    }
}
