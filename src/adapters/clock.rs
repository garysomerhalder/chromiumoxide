//! Tokio-backed clock.

use std::sync::OnceLock;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::ports::Clock;

/// Clock using tokio's timer.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioClock;

impl Clock for TokioClock {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        tokio::time::sleep(duration).boxed()
    }

    fn wall_ms(&self) -> f64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }

    fn now_ms(&self) -> u64 {
        static START: OnceLock<tokio::time::Instant> = OnceLock::new();
        let start = *START.get_or_init(tokio::time::Instant::now);
        tokio::time::Instant::now()
            .duration_since(start)
            .as_millis() as u64
    }
}
