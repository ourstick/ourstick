//! Waiting for the owner of a link the app waits for (ADR 0025): the
//! receive's own pauses and reasons, until the owner gives its preview.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::super::{preview, DownloadError, Preview};
use super::{after_error, pause, Cancel, Next, Waiting};
use crate::store::Store;
use crate::ticket::Ticket;
use crate::transport::{Fetcher, TransportError};

/// Asks the owner for the stick's preview (into the folder `choose` gives
/// its name) until it answers, each try bounded by `attempt` (an owner who
/// is away may never answer at all). Between tries it waits as a receive
/// does ([`after_error`]: 15 s, doubling to 5 minutes), telling `on_wait`
/// why each time. Ends with the preview, with [`DownloadError::Cancelled`]
/// once `cancel`, or with an error that needs the user.
pub async fn wait_for_owner<F: Fetcher>(
    fetcher: &F,
    store: Arc<dyn Store>,
    ticket: &Ticket,
    choose: impl Fn(&str) -> PathBuf + Clone + Send + 'static,
    attempt: Duration,
    cancel: &Cancel,
    on_wait: &mut (dyn FnMut(&Waiting) + Send),
) -> Result<Preview, DownloadError> {
    let mut waited = Duration::ZERO;
    let mut last_why: Option<std::mem::Discriminant<Waiting>> = None;
    loop {
        let tried = tokio::select! {
            tried = tokio::time::timeout(
                attempt,
                preview(fetcher, store.clone(), ticket, choose.clone()),
            ) => tried,
            () = cancel.cancelled() => return Err(DownloadError::Cancelled),
        };
        let error = match tried {
            Ok(Ok(preview)) => return Ok(preview),
            Ok(Err(error)) => error,
            Err(_) => DownloadError::Transport(TransportError::Unreachable("no answer".to_owned())),
        };
        let mut next = after_error(&error, None, waited);
        // A new reason starts from the first pause again, as in a receive.
        if let Next::Wait(why, _) = &next {
            let why = std::mem::discriminant(why);
            if last_why != Some(why) {
                last_why = Some(why);
                next = after_error(&error, None, Duration::ZERO);
            }
        }
        match next {
            Next::Stop => return Err(error),
            Next::Wait(why, after) => {
                on_wait(&why);
                waited = after;
                if !pause(after, cancel).await {
                    return Err(DownloadError::Cancelled);
                }
            }
        }
    }
}
