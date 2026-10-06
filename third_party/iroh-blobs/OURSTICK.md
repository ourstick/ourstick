# iroh-blobs 0.103.1, patched

This is [iroh-blobs](https://github.com/n0-computer/iroh-blobs) 0.103.1 as
published on crates.io (MIT or Apache-2.0, see `LICENSE-MIT` and
`LICENSE-APACHE`), with one change, in `src/store/fs.rs`: `RtWrapper::drop`
shuts the store's runtime down with `shutdown_background()` instead of
dropping it under `block_in_place`.

- Vendored: 2026-10-06
- Changed file: `src/store/fs.rs` (`impl Drop for RtWrapper`). `Cargo.toml`
  lost its example and test targets and dev-dependencies, since only `src/`
  (and `build.rs`, and `docs/`, which its doc comments include) is kept.

Why (ADR 0018): an `FsStore`'s actor owns the store's tokio runtime and runs
on it. When the last `FsStore` handle goes, the actor ends and drops the
runtime on one of that runtime's own threads; the drop waits for every
thread of the runtime to end, that one included, and so waits forever. Each
closed store left two or three `iroh-blob-store` threads behind: the reader
opens one store per batch, and reached 402 threads for 100 000 files.

Used through `[patch.crates-io]` in the workspace `Cargo.toml`. Reported
upstream (see `docs/upstream/`); once a release has the fix, delete this
folder and the patch entry.
