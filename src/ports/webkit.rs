//! Playwright's WebKit protocol port (the `--inspector-pipe` of its WebKit
//! build): browser commands, page-proxy commands and page-target commands.

use futures::future::BoxFuture;
use serde_json::Value;

use crate::domain::error::Result;
use crate::ports::cdp::EventStream;

/// A connection to Playwright's WebKit. Events arrive as
/// [`crate::ports::RawEvent`]s whose `session_id` is `None` for the browser,
/// `Some("<pageProxyId>")` for a page proxy and
/// `Some("<pageProxyId>/<targetId>")` for a page target.
pub trait WebKitSession: Send + Sync {
    /// Sends a browser command (`page_proxy: None`, e.g. `Playwright.*`) or a
    /// page-proxy command (`Emulation.*`, `Target.*`, `Input.*`).
    fn call(
        &self,
        page_proxy: Option<&str>,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>>;

    /// Sends a command to a page target (`Page.*`, `Runtime.*`, `DOM.*`,
    /// `Network.*`), wrapped in `Target.sendMessageToTarget`.
    fn call_target(
        &self,
        page_proxy: &str,
        target: &str,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>>;

    /// Every event received from now on.
    fn subscribe(&self) -> EventStream;

    /// Closes the connection.
    fn disconnect(&self) {}
}
