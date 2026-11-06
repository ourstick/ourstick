//! A link on the clipboard that the clipboard does not keep: copied for
//! someone else, a stick's key among it, it stays out of the clipboard's
//! history and sync where the system has a rule for that.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::put_text;
#[cfg(windows)]
pub use windows::put_text;
