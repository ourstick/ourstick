//! The threads of a program that shares or receives sticks.
//!
//! iroh-blobs gives each of its file stores its own tokio runtime, with a
//! worker thread per core and no setting for it; the owner opens a store per
//! suitcase (ADR 0007) and a reader one per batch, several at once. On a
//! 32-core PC the owner of 100 000 small files ran about 290 threads, each
//! with its own stack and its own part of the allocator, which keeps memory
//! freed: the program grew to 420 MB with 80 MB in use. Those runtimes read
//! `TOKIO_WORKER_THREADS`, so it is set before any runtime starts. The
//! program's own runtime is kept small too: its work is waiting on the
//! network, and what takes a core (reading, hashing) runs on blocking
//! threads.

/// Worker threads of each iroh-blobs store: they only move blobs between
/// the disk and the network. Measured (10 000 small files, one PC): 4 made
/// a download 20–50 % slower, 8 about as fast as one per core.
pub const STORE_WORKER_THREADS: usize = 8;

/// At most this many worker threads in the program's own runtime.
pub const PROGRAM_WORKER_THREADS: usize = 4;

/// Limits the worker threads of every tokio runtime made without an explicit
/// count to [`STORE_WORKER_THREADS`], or the cores if fewer (a valid value
/// the user set is kept), and returns how many the program's own runtime
/// should have: one per core, at most [`PROGRAM_WORKER_THREADS`]. Call it
/// first in `main`, before any thread starts.
pub fn limit_store_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let set = std::env::var("TOKIO_WORKER_THREADS").ok();
    if let Some(threads) = store_threads_to_set(set.as_deref(), cores) {
        std::env::set_var("TOKIO_WORKER_THREADS", threads.to_string());
    }
    cores.min(PROGRAM_WORKER_THREADS)
}

/// What to set `TOKIO_WORKER_THREADS` to, given what it is: `None` keeps a
/// positive number the user set; anything else (unset, 0, not a number,
/// which would make tokio panic at the first store) is replaced.
fn store_threads_to_set(set: Option<&str>, cores: usize) -> Option<usize> {
    match set.and_then(|value| value.trim().parse::<usize>().ok()) {
        Some(threads) if threads > 0 => None,
        _ => Some(STORE_WORKER_THREADS.min(cores).max(1)),
    }
}

/// The program's own runtime, with `worker_threads` workers (see
/// [`limit_store_threads`]).
pub fn program_runtime(worker_threads: usize) -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    // The variable itself is not set here: tests run in parallel, and
    // others would see it change under them.
    #[test]
    fn stores_get_few_threads_unless_the_user_chose() {
        assert_eq!(store_threads_to_set(None, 32), Some(STORE_WORKER_THREADS));
        assert_eq!(store_threads_to_set(None, 2), Some(2));
        assert_eq!(store_threads_to_set(Some("3"), 32), None);
        assert_eq!(
            store_threads_to_set(Some("0"), 32),
            Some(STORE_WORKER_THREADS)
        );
        assert_eq!(
            store_threads_to_set(Some("many"), 32),
            Some(STORE_WORKER_THREADS)
        );
    }

    #[test]
    fn the_program_runtime_has_the_workers_asked() {
        let runtime = program_runtime(2).unwrap();
        assert_eq!(runtime.metrics().num_workers(), 2);
    }
}
