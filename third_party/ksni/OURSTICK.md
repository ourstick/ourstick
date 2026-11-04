# ksni 0.3.6, patched

This is [ksni](https://github.com/iovxw/ksni) 0.3.6 as published on crates.io
(Unlicense, see `UNLICENSE`), with one change: the StatusNotifierItem method
`ProvideXdgActivationToken` and `Tray::provide_xdg_activation_token`.

- Upstream pull request: https://github.com/iovxw/ksni/pull/43
  (opened 2026-09-29, commit 2576acae on KalGeo/ksni `provide-xdg-activation-token`)
- Vendored: 2026-09-29
- Changed files: `src/lib.rs`, `src/service.rs`, `src/dbus_interface.rs`. The
  pull request also adds a protocol test, the method in the spec XML and a
  changelog entry; they are not copied here. `Cargo.toml` lost its example
  and test targets and dev-dependencies, since only `src/` is kept.

Why: GNOME's AppIndicator extension passes an XDG activation token to the
tray item before each click. On Wayland it is what lets the app bring its own
window to the front; without it GNOME shows "OurStick is ready" instead (see
`docs/gui-spec.md`, Known limitations).

Used through `[patch.crates-io]` in the workspace `Cargo.toml`. Once a ksni
release has the method, delete this folder and the patch entry.

`OURSTICK.patch` is the whole difference from the published crate, and CI
checks it (`tools/third_party/check.sh`; `--write` after a deliberate
change).
