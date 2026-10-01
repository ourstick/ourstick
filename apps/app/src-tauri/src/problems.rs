//! The problems the window shows: the last one of each stick, and of the
//! whole app, kept here (not in the window) so that none is lost when
//! another stick is shown or the window is not open yet. The window hears
//! `problems-changed` and asks again.

use std::collections::HashMap;
use std::sync::Mutex;

use ourstick_core::StickId;
use tauri::{AppHandle, Emitter, Manager};

use crate::serving::ShareProblem;

/// The last problem of each stick (`Some`) and of the whole app (`None`).
#[derive(Default)]
pub struct Problems(Mutex<HashMap<Option<StickId>, ShareProblem>>);

impl Problems {
    /// Records `problem` (`Cleared` forgets it); whether anything changed.
    pub fn record(&self, about: Option<StickId>, problem: ShareProblem) -> bool {
        let mut problems = self.0.lock().expect("problems lock");
        match problem {
            ShareProblem::Cleared => problems.remove(&about).is_some(),
            problem => problems.insert(about, problem.clone()).as_ref() != Some(&problem),
        }
    }

    pub fn of(&self, about: Option<&StickId>) -> Option<ShareProblem> {
        self.0
            .lock()
            .expect("problems lock")
            .get(&about.copied())
            .cloned()
    }
}

/// Tells the window about a problem of `about` (a stick, or the whole app
/// with `None`).
pub fn tell(app: &AppHandle, about: Option<StickId>, problem: ShareProblem) {
    if let ShareProblem::Failed { message } = &problem {
        tracing::warn!(%message, "a problem shown in the window");
    }
    if app.state::<Problems>().record(about, problem) {
        let _ = app.emit("problems-changed", ());
    }
}

/// Tells of a failure of `about`.
pub fn failed(app: &AppHandle, about: Option<StickId>, message: String) {
    tell(app, about, ShareProblem::Failed { message });
}

/// The whole app's problem, read by the window ([`tell`] with `None`).
#[tauri::command]
pub fn app_problem(app: AppHandle) -> Option<ShareProblem> {
    app.state::<Problems>().of(None)
}

/// The user closed the app's problem.
#[tauri::command]
pub fn dismiss_app_problem(app: AppHandle) {
    tell(&app, None, ShareProblem::Cleared);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_stick_keeps_its_own_last_problem() {
        let problems = Problems::default();
        let photos = ourstick_core::StickSigningKey::from_bytes(&[1; 32]).stick_id();
        let films = ourstick_core::StickSigningKey::from_bytes(&[2; 32]).stick_id();
        assert!(problems.record(Some(photos), ShareProblem::Ejected));
        assert!(!problems.record(Some(photos), ShareProblem::Ejected));
        assert!(problems.record(
            None,
            ShareProblem::Failed {
                message: "no keyring".to_owned()
            }
        ));
        assert_eq!(problems.of(Some(&photos)), Some(ShareProblem::Ejected));
        assert_eq!(problems.of(Some(&films)), None);
        assert!(problems.record(Some(photos), ShareProblem::Cleared));
        assert!(!problems.record(Some(photos), ShareProblem::Cleared));
        assert_eq!(problems.of(Some(&photos)), None);
        assert!(problems.of(None).is_some());
    }
}
