//! Process signals with tokio.

use futures::StreamExt;
use futures::stream::BoxStream;

use crate::ports::signals::{Signal, Signals};

/// Signals of the running process.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsSignals;

impl Signals for OsSignals {
    fn listen(&self, signal: Signal) -> BoxStream<'static, ()> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal as unix_signal};
            let kind = match signal {
                Signal::Interrupt => SignalKind::interrupt(),
                Signal::Terminate => SignalKind::terminate(),
                Signal::Hangup => SignalKind::hangup(),
            };
            match unix_signal(kind) {
                Ok(s) => futures::stream::unfold(s, |mut s| async move {
                    s.recv().await?;
                    Some(((), s))
                })
                .boxed(),
                Err(e) => {
                    tracing::warn!(target: "chromiumoxide", "cannot listen for {signal:?}: {e}");
                    futures::stream::empty().boxed()
                }
            }
        }
        #[cfg(not(unix))]
        match signal {
            Signal::Interrupt => futures::stream::unfold((), |()| async {
                tokio::signal::ctrl_c().await.ok()?;
                Some(((), ()))
            })
            .boxed(),
            _ => futures::stream::empty().boxed(),
        }
    }

    fn exit(&self, code: i32) -> ! {
        std::process::exit(code)
    }
}
