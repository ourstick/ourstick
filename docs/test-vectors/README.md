# Test vectors

Known answers for OurStick's formats, so that another implementation can
check itself against this one: blobs (`blob-v1.json`), manifests
(`manifest-v2.json`), tickets and signed records (`ticket-v1.json`). Each is
made by the independent Python script beside it and checked by the Rust
tests in `crates/ourstick-core/tests/*_vectors.rs`. The formats are described
in `docs/protocol.md`.

Every key, seed and nonce here is public on purpose: made up in the
scripts (bytes counting from `00`, a fixed hex string, the BLAKE3 hash of
the words "stick key") or derived from those, never taken from a real stick
or device. They open nothing.
