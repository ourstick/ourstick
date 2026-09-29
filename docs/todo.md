# To do (not code)

Things that are waiting on something other than code. Code work belongs in
the implementation order in `CLAUDE.md`; limits of the code are in
`docs/protocol.md` ("Known limits") and `docs/gui-spec.md`.

## GitHub

- [ ] Make the GitHub organisation (`ourstick`, as in `repository` in
      `Cargo.toml`) and push the repository there. Today it lives only on the
      private `hetzner` remote.

## Upstream

- [ ] **iroh-blobs: GC logs `error during gc run: Error::Io` after the store
      shuts down, and stops for good after any error.** A public issue; the
      draft (with a standalone reproduction) is kept outside the repository.
      In OurStick the line appears once after each Stop sharing, Mount,
      Ctrl-C or Quit and is harmless (`docs/protocol.md`, "Known limits").
- [ ] **ksni: `ProvideXdgActivationToken`,**
      https://github.com/iovxw/ksni/pull/43 (opened 2026-09-29). Answer the
      review if there is one. Once a ksni release has the method, drop
      `third_party/ksni` and the `[patch.crates-io]` entry
      (`third_party/ksni/OURSTICK.md`).

## Before the first public release

- [ ] **Our own relay, `relay.ourstick.com`,** first in the relay list with
      the n0 relays as a fallback (`docs/decisions/0006-relay-policy.md`).
- [ ] **`https://ourstick.com/.well-known/assetlinks.json`,** so that Android
      opens `https://ourstick.com/s/…` links in the app. The file names the
      app's package and the SHA-256 fingerprint of its signing certificate, so
      it needs the release signing key first.
- [ ] The macOS signing and the other items in `docs/gui-spec.md`, "Before
      the first public release".
