//! Sharing a connection: `browser.bind()` serves the browser's CDP
//! connection to other clients over a WebSocket.

use std::io;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::ports::CdpSession;

/// Where a bridge listens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BridgeAddress {
    /// Default `127.0.0.1`.
    pub host: Option<String>,
    /// Default `0`: any free port.
    pub port: Option<u16>,
    /// The only path clients may use (default: an unguessable one).
    pub path: Option<String>,
}

/// Starts serving a CDP connection.
pub trait CdpBridge: Send + Sync {
    fn serve(
        &self,
        cdp: Arc<dyn CdpSession>,
        address: BridgeAddress,
    ) -> BoxFuture<'static, io::Result<Box<dyn BoundBridge>>>;
    /// Whether something accepts connections at a bridge `endpoint`
    /// (a `ws://host:port/...` URL).
    fn is_listening(&self, endpoint: &str) -> BoxFuture<'static, bool>;
}

/// A running bridge.
pub trait BoundBridge: Send + Sync {
    /// The WebSocket URL clients connect to.
    fn endpoint(&self) -> String;
    /// Stops accepting clients and drops the connected ones.
    fn stop(&self);
}
