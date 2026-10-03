//! A received stick's reports to its owner (ADR 0015): the reporter beside
//! each receive, and the one report a pause or a forget sends.

use std::sync::Arc;
use std::time::Duration;

use ourstick_core::clock::SystemClock;
use ourstick_core::device::{open_system_keyring, random_key};
use ourstick_core::download::Reporter;
use ourstick_core::received::{reader_key, received_store_dir};
use ourstick_core::report::ReportState;
use ourstick_core::transport::iroh::{IrohConfig, IrohFetcher, Network};
use ourstick_core::transport::Fetcher;
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

/// Tells the owner `state` (paused, forgotten) once, if it can be reached
/// within [`ENDING_DEADLINE`]; about the version last received whole.
/// Never fails: the owner may well be away.
pub(super) async fn report_once(app: &AppHandle, stick_id: StickId, state: ReportState) {
    let told = tokio::time::timeout(ENDING_DEADLINE, async {
        let app_state = app.state::<AppState>();
        let store = app_state.store.clone();
        let (ticket, seq) = super::work(move || {
            let secrets = open_system_keyring()?;
            let ticket = super::read_ticket(secrets.as_ref(), &stick_id)?;
            let seq = store
                .joined_stick(&stick_id)?
                .and_then(|joined| joined.completed)
                .map_or(0, |completed| completed.seq);
            Ok((ticket, seq))
        })
        .await
        .ok()?;
        let reporter = reporter_for(app, stick_id, ticket).await?;
        reporter.set_version(seq);
        let config = IrohConfig::new(random_key(), Network::Public);
        let store_dir = received_store_dir(&app_state.data_dir, &stick_id);
        let fetcher = IrohFetcher::start(&config, &store_dir).await.ok()?;
        let sent = reporter.send_now(&fetcher, state, ENDING_DEADLINE).await;
        let _ = fetcher.shutdown(true).await;
        sent.ok()
    })
    .await;
    if !matches!(told, Ok(Some(()))) {
        tracing::debug!("the owner was not told of a pause or a forget");
    }
}
