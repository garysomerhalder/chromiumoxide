//! Clock port, for timeouts and polling.

use std::time::Duration;

use futures::future::BoxFuture;

/// Time source.
pub trait Clock: Send + Sync {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()>;
    /// Monotonic milliseconds since an arbitrary origin.
    fn now_ms(&self) -> u64;
    /// Wall-clock milliseconds since the Unix epoch (for trace and HAR times).
    fn wall_ms(&self) -> f64 {
        0.0
    }
}
