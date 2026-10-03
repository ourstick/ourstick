//! A received stick's reports to its owner (ADR 0015): the reporter beside
//! each receive, and the one report a pause or a forget sends.

use std::sync::Arc;
use std::time::Duration;

use ourstick_core::clock::SystemClock;
use ourstick_core::device::{open_system_keyring, random_key};
use ourstick_core::download::Reporter;
use ourstick_core::received::{reader_key, reader_key_if_any};
use ourstick_core::report::ReportState;
use ourstick_core::transport::iroh::{IrohConfig, IrohReportSender, Network};
use ourstick_core::{StickId, Ticket};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

/// How long a pause or a forget waits for its report to reach the owner.
pub const ENDING_DEADLINE: Duration = Duration::from_secs(3);

/// The reporter for a received stick, with this computer's reader key for
/// it (made and kept the first time) and the name its receiver chose, if
/// any. `None` if the key cannot be had: reports never stop a receive.
pub(super) async fn reporter_for(
    app: &AppHandle,
    stick_id: StickId,
    ticket: Ticket,
) -> Option<Arc<Reporter>> {
    let store = app.state::<AppState>().store.clone();
    let kept = store.clone();
    let made = super::work(move || {
        let secrets = open_system_keyring()?;
        Ok((
            reader_key(secrets.as_ref(), &stick_id)?,
            kept.receiver_name()?,
        ))
    })
    .await;
    match made {
        Ok((key, name)) => Some(Arc::new(Reporter::new(
            ticket,
            key,
            name,
            store,
            Arc::new(SystemClock),
        ))),
        Err(why) => {
            tracing::warn!(%why, "a received stick reports nothing to its owner");
            None
        }
    }
}

/// Tells the owner `state` (paused, forgotten) once, if this computer
/// ever reported to it and it can be reached within [`ENDING_DEADLINE`];
/// about the version last received whole. With an endpoint of its own and
/// no blob store, so the stick's store is never held twice. Never fails:
/// the owner may well be away.
pub(super) async fn report_once(app: &AppHandle, stick_id: StickId, state: ReportState) {
    let app_state = app.state::<AppState>();
    let store = app_state.store.clone();
    let known = super::work(move || {
        let secrets = open_system_keyring()?;
        let Some(key) = reader_key_if_any(secrets.as_ref(), &stick_id)? else {
            return Ok(None);
        };
        let ticket = super::read_ticket(secrets.as_ref(), &stick_id)?;
        let seq = store
            .joined_stick(&stick_id)?
            .and_then(|joined| joined.completed)
            .map_or(0, |completed| completed.seq);
        Ok(Some((key, ticket, seq, store.receiver_name()?)))
    })
    .await;
    let Ok(Some((key, ticket, seq, name))) = known else {
        return;
    };
    let reporter = Reporter::new(
        ticket,
        key,
        name,
        app_state.store.clone(),
        Arc::new(SystemClock),
    );
    reporter.set_version(seq);
    let config = IrohConfig::new(random_key(), Network::Public);
    let Ok(sender) = IrohReportSender::start(&config).await else {
        return;
    };
    if reporter
        .send_now(&sender, state, ENDING_DEADLINE)
        .await
        .is_err()
    {
        tracing::debug!("the owner was not told of a pause or a forget");
    }
    sender.close().await;
}

/// How senders see this computer: its kind, which every report says, and
/// the name its receiver chose, if any.
#[derive(Debug, serde::Serialize)]
pub struct SeenAs {
    /// "macos", "windows", "linux", "other", as a report says it.
    kind: &'static str,
    name: Option<String>,
}

#[tauri::command]
pub async fn receiver_name(state: tauri::State<'_, AppState>) -> Result<SeenAs, String> {
    let store = state.store.clone();
    let name = super::work(move || Ok(store.receiver_name()?)).await?;
    let kind = match ourstick_core::report::ComputerKind::this() {
        ourstick_core::report::ComputerKind::MacOs => "macos",
        ourstick_core::report::ComputerKind::Windows => "windows",
        ourstick_core::report::ComputerKind::Linux => "linux",
        ourstick_core::report::ComputerKind::Other => "other",
    };
    Ok(SeenAs { kind, name })
}

/// Sets the name senders see, or clears it (empty). Never asked for: the
/// receiver chooses to (ADR 0015, rule 4).
#[tauri::command]
pub async fn set_receiver_name(
    name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let store = state.store.clone();
    super::work(move || {
        let name = name.trim();
        let name = (!name.is_empty()).then_some(name);
        if name.is_some_and(|name| !ourstick_core::report::name_is_allowed(name)) {
            anyhow::bail!("A name is at most 40 characters, with no invisible ones.");
        }
        Ok(store.set_receiver_name(name)?)
    })
    .await
}
