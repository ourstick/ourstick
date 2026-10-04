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
/// count to [`STORE_WORKER_THREADS`] (a value the user set is kept), and
/// returns how many the program's own runtime should have: one per core, at
/// most [`PROGRAM_WORKER_THREADS`]. Call it first in `main`, before any
/// thread starts.
pub fn limit_store_threads() -> usize {
    if std::env::var_os("TOKIO_WORKER_THREADS").is_none() {
        std::env::set_var("TOKIO_WORKER_THREADS", STORE_WORKER_THREADS.to_string());
    }
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(PROGRAM_WORKER_THREADS)
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

    #[test]
    fn stores_get_few_threads_and_the_program_one_per_core() {
        let own = limit_store_threads();
        assert!((1..=PROGRAM_WORKER_THREADS).contains(&own));
        let set = std::env::var("TOKIO_WORKER_THREADS").unwrap();
        assert!(set.parse::<usize>().unwrap() >= 1);
        let runtime = program_runtime(2).unwrap();
        assert_eq!(runtime.metrics().num_workers(), 2);
    }
}
