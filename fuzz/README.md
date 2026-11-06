# Fuzzing

Fuzz targets for what OurStick reads from outside, run with
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer). This
folder is its own workspace: it is never built with the app.

| Target     | What it reads                                                                 |
|------------|-------------------------------------------------------------------------------|
| `ticket`   | a pasted link or bare ticket; a ticket read is written back the same          |
| `manifest` | a sender's manifest (the sender holds the key); it encodes back the same      |
| `record`   | a signed manifest record; never accepted without the stick's signature        |
| `report`   | a reader's report (a reader holds the key): any report sealed opens the same  |
| `blob`     | a sender's blob, both ciphers; plaintext encrypts and decrypts to itself      |
| `notice`   | `version.json` from anyone in the way; never a panic                          |
| `names`    | a peer's text shown to the user: escaped, no control or direction characters |
| `saved_paths` | a sender's paths as saved: savable everywhere, none of ours, no two in one place |

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
cd fuzz
cargo +nightly run --example make-seeds      # valid inputs, throwaway keys
cargo +nightly fuzz run manifest -- -max_total_time=600
```

An input that fails is written to `artifacts/<target>/`; once fixed, it
becomes a test beside the code it found wrong.

## Runs

| Day        | Each target | Inputs run, all targets | Failures |
|------------|-------------|-------------------------|----------|
| 2026-11-06 | 3 hours     | 3 440 million           | none     |
