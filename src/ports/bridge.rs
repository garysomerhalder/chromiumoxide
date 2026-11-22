//! Sharing a connection: `browser.bind()` serves the browser's CDP
//! connection to other clients over a WebSocket.

use std::io;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::ports::CdpSession;

/// Starts serving a CDP connection.
pub trait CdpBridge: Send + Sync {
    fn serve(
        &self,
        cdp: Arc<dyn CdpSession>,
    ) -> BoxFuture<'static, io::Result<Box<dyn BoundBridge>>>;
}

/// A running bridge.
pub trait BoundBridge: Send + Sync {
    /// The WebSocket URL clients connect to.
    fn endpoint(&self) -> String;
    /// Stops accepting clients and drops the connected ones.
    fn stop(&self);
}
