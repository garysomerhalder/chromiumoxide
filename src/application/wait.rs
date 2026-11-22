//! Waiting on page state with a timeout.

use std::time::Duration;

use futures::FutureExt;
use tokio::sync::watch;

use crate::domain::error::{Error, Result};
use crate::ports::Clock;

/// Re-evaluates `check` whenever `changed` ticks, until it yields a value,
/// the timeout elapses, or `closed` reports the target went away.
pub async fn wait_until<T>(
    clock: &dyn Clock,
    changed: &watch::Sender<u64>,
    timeout: Duration,
    action: &str,
    mut check: impl FnMut() -> Option<Result<T>>,
) -> Result<T> {
    let mut rx = changed.subscribe();
    let mut deadline = clock.sleep(timeout).fuse();
    loop {
        if let Some(result) = check() {
            return result;
        }
        let tick = rx.changed().fuse();
        futures::pin_mut!(tick);
        futures::select! {
            r = tick => {
                if r.is_err() {
                    return Err(Error::TargetClosed(format!("closed while {action}")));
                }
            }
            _ = deadline => {
                return Err(Error::Timeout { ms: timeout.as_millis() as u64, action: action.to_owned() });
            }
        }
    }
}

/// Runs `fut` with a timeout.
pub async fn with_timeout<T>(
    clock: &dyn Clock,
    timeout: Duration,
    action: &str,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let fut = fut.fuse();
    let deadline = clock.sleep(timeout).fuse();
    futures::pin_mut!(fut, deadline);
    futures::select! {
        r = fut => r,
        _ = deadline => Err(Error::Timeout { ms: timeout.as_millis() as u64, action: action.to_owned() }),
    }
}
