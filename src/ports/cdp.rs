//! Protocol session port: CDP request/response and events.

use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::domain::error::Result;

/// A protocol event as received.
#[derive(Debug, Clone, PartialEq)]
pub struct RawEvent {
    /// Flat-mode session the event belongs to (`None`: browser target).
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

/// Stream of events, in arrival order.
pub type EventStream = Pin<Box<dyn Stream<Item = Arc<RawEvent>> + Send>>;

/// A CDP connection: calls and an event feed (flat sessions).
pub trait CdpSession: Send + Sync {
    /// Sends `method` with `params`, to `session_id` if given; resolves with
    /// the `result` object.
    fn call(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>>;

    /// Every event received from now on. Subscribe *before* issuing the call
    /// that triggers the events you need.
    fn subscribe(&self) -> EventStream;

    /// Closes the connection (the browser keeps running).
    fn disconnect(&self) {}
}
