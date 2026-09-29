//! Keeping the computer awake while someone downloads from the stick
//! (docs/gui-spec.md, "Sleep"): a sleeping owner stops sharing mid-download.

use anyhow::Result;

/// While held, the system does not go to sleep on its own.
pub struct SleepGuard {
    #[cfg(target_os = "linux")]
    _lock: zbus::zvariant::OwnedFd,
    #[cfg(target_os = "macos")]
    caffeinate: std::process::Child,
}

impl SleepGuard {
    /// Linux: a logind "sleep" inhibitor lock, held as long as its file
    /// descriptor is open.
    #[cfg(target_os = "linux")]
    pub async fn acquire() -> Result<Self> {
        const WHY: &str = "Someone is downloading from your OurStick";
        let connection = zbus::Connection::system().await?;
        let reply = connection
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &("sleep", "OurStick", WHY, "block"),
            )
            .await?;
        let lock: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
        Ok(Self { _lock: lock })
    }

    /// macOS: `caffeinate -i` for as long as it runs; `-w` ends it with the
    /// app too, should the app die without dropping the guard.
    #[cfg(target_os = "macos")]
    pub async fn acquire() -> Result<Self> {
        let caffeinate = std::process::Command::new("caffeinate")
            .arg("-i")
            .arg("-w")
            .arg(std::process::id().to_string())
            .spawn()?;
        Ok(Self { caffeinate })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub async fn acquire() -> Result<Self> {
        anyhow::bail!("keeping the system awake is not done on this system yet")
    }
}

#[cfg(target_os = "macos")]
impl Drop for SleepGuard {
    fn drop(&mut self) {
        let _ = self.caffeinate.kill();
        let _ = self.caffeinate.wait();
    }
}

/// Holds a guard while `downloading`, and lets go of it when not.
#[derive(Default)]
pub struct KeepAwake {
    guard: Option<SleepGuard>,
}

impl KeepAwake {
    pub async fn set(&mut self, downloading: bool) {
        match (downloading, self.guard.is_some()) {
            (true, false) => match SleepGuard::acquire().await {
                Ok(guard) => self.guard = Some(guard),
                // Not fatal: sharing works, the computer may just sleep.
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "cannot keep the computer awake")
                }
            },
            (false, true) => self.guard = None,
            _ => {}
        }
    }
}
