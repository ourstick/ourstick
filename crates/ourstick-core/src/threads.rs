//! The threads of a program that shares or receives sticks.
//!
//! iroh-blobs gives each of its file stores its own tokio runtime, with a
//! worker thread per core and no setting for it; a reader opens one per
//! batch (the owner did one per suitcase until ADR 0017). On a
//! 32-core PC the owner of 100 000 small files ran about 290 threads, each
//! with its own stack and its own part of the allocator, which keeps memory
//! freed: the program grew to 420 MB with 80 MB in use. Those runtimes read
//! `TOKIO_WORKER_THREADS`, so it is set before any runtime starts. The
//! program's own runtime is kept small too: its work is waiting on the
//! network, and what takes a core (reading, hashing) runs on blocking
//! threads.
//!
//! On Linux, glibc's allocator also gives each thread that allocates an
//! arena of its own (up to eight per core), and each keeps much of what is
//! freed in it. With ADR 0017 the owner of 100 000 small files, its heap
//! at most 176 MB, still grew from 282 to 311 MB while it sent them all;
//! with two arenas (`MALLOC_ARENA_MAX=2`) it stayed at 220–223 MB from the
//! first minute to the end, and the download was no slower (2026-10-05).
//! That setting is read when the program starts, so it is made with
//! `mallopt` instead, first thing.

/// Worker threads of each iroh-blobs store: they only move blobs between
/// the disk and the network. Measured (10 000 small files, one PC): 4 made
/// a download 20–50 % slower, 8 about as fast as one per core.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub const STORE_WORKER_THREADS: usize = 8;

/// A phone (ADR 0027) gets one stick at a time, with less memory and a
/// battery: fewer.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub const STORE_WORKER_THREADS: usize = 4;

/// At most this many worker threads in the program's own runtime.
pub const PROGRAM_WORKER_THREADS: usize = 4;

/// Limits glibc's arenas ([`limit_malloc_arenas`]) and the worker threads of
/// every tokio runtime made without an explicit
/// count to [`STORE_WORKER_THREADS`], or the cores if fewer (a valid value
/// the user set is kept), and returns how many the program's own runtime
/// should have: one per core, at most [`PROGRAM_WORKER_THREADS`]. Call it
/// first in `main`, before any thread starts.
pub fn limit_store_threads() -> usize {
    limit_malloc_arenas();
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let set = std::env::var("TOKIO_WORKER_THREADS").ok();
    if let Some(threads) = store_threads_to_set(set.as_deref(), cores) {
        std::env::set_var("TOKIO_WORKER_THREADS", threads.to_string());
    }
    cores.min(PROGRAM_WORKER_THREADS)
}

/// glibc's arenas, at most this many (see the module's notes).
pub const MALLOC_ARENAS: i32 = 2;

/// Limits glibc's allocator to [`MALLOC_ARENAS`] arenas; true if it took
/// the setting. Nothing to do elsewhere (macOS, Windows and musl have other
/// allocators).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub fn limit_malloc_arenas() -> bool {
    /// glibc's `M_ARENA_MAX` (malloc.h), the same since glibc 2.10.
    const M_ARENA_MAX: std::ffi::c_int = -8;
    extern "C" {
        /// `int mallopt(int param, int value)`, glibc's malloc.h.
        fn mallopt(param: std::ffi::c_int, value: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: `mallopt` is declared as glibc's malloc.h has it (two ints in,
    // an int out) and takes no pointer: it reads or writes no memory of
    // ours. glibc takes its arena locks itself, so a call while other
    // threads allocate is allowed; it is made first in `main` anyway. On
    // failure it returns 0 and changes nothing.
    unsafe { mallopt(M_ARENA_MAX, MALLOC_ARENAS) == 1 }
}

/// Limits glibc's allocator; elsewhere there is nothing to do.
#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
pub fn limit_malloc_arenas() -> bool {
    false
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
    fn glibc_takes_the_arena_limit() {
        let took = limit_malloc_arenas();
        assert_eq!(took, cfg!(all(target_os = "linux", target_env = "gnu")));
    }

    #[test]
    fn the_program_runtime_has_the_workers_asked() {
        let runtime = program_runtime(2).unwrap();
        assert_eq!(runtime.metrics().num_workers(), 2);
    }
}
