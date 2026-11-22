//! Clock backed by tokio time (use `#[tokio::test(start_paused = true)]` to control it).

use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::ports::Clock;

#[derive(Default, Clone)]
pub struct TestClock;

impl Clock for TestClock {
    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        tokio::time::sleep(d).boxed()
    }
    fn now_ms(&self) -> u64 {
        static START: std::sync::OnceLock<tokio::time::Instant> = std::sync::OnceLock::new();
        let start = *START.get_or_init(tokio::time::Instant::now);
        tokio::time::Instant::now()
            .duration_since(start)
            .as_millis() as u64
    }
}
