//! Clock port, for timeouts and polling.

use std::time::Duration;

use futures::future::BoxFuture;

/// Time source.
pub trait Clock: Send + Sync {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()>;
    /// Monotonic milliseconds since an arbitrary origin.
    fn now_ms(&self) -> u64;
}
