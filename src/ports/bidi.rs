//! WebDriver BiDi session port: commands and events (Firefox).

use futures::future::BoxFuture;
use serde_json::Value;

use crate::domain::error::Result;
use crate::ports::cdp::EventStream;

/// A WebDriver BiDi connection. Events arrive as [`crate::ports::RawEvent`]s
/// with no session id.
pub trait BidiSession: Send + Sync {
    /// Sends `method` with `params`; resolves with the `result` object, or
    /// [`crate::domain::error::Error::Protocol`] with the BiDi error code
    /// and message.
    fn call(&self, method: &str, params: Value) -> BoxFuture<'static, Result<Value>>;

    /// Every event received from now on.
    fn subscribe(&self) -> EventStream;

    /// Closes the connection (the browser keeps running).
    fn disconnect(&self) {}
}
