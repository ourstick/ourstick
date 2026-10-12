# Contributing

Thank you for looking. OurStick is made by one person for now, so the most
useful things are reports from real use:

- **A bug or an idea:** open an issue, or write to **hi@ourstick.com**. Say
  which system and which version (the app shows it at the foot of its
  window, `ourstick --version` for the CLI), what you did and what happened.
- **A security problem:** never as an issue; see [SECURITY.md](SECURITY.md).
- **Never paste a real sharing link**, in an issue, a log or a screenshot:
  a link carries the stick's key and the sender's network addresses. Make a
  test stick, and remove it after.

## Code

Please open an issue before a large change: the design is settled in
[docs/decisions](docs/decisions), and a change that goes against one of them
starts with a new decision. How to build and test is in the
[README](README.md#development); the project's rules (in Greek) are in
[CLAUDE.md](CLAUDE.md). In short:

- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace` pass;
- every public function has a test; `ourstick-core` knows nothing of the
  app or the CLI;
- no ticket or key is ever logged;
- small commits, as [conventional commits](https://www.conventionalcommits.org/),
  in English.

Contributions are accepted under the project's licence, Apache-2.0
(section 5 of the [LICENSE](LICENSE)).
