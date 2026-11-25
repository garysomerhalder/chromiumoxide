//! Typed WebKit commands (`chromiumoxide_webkit_protocol`, generated from
//! WebKit's `protocol.json`) over the [`WebKitSession`] port.

use chromiumoxide_types::Command;

use crate::domain::error::{Error, Result};
use crate::ports::WebKitSession;

pub(crate) use chromiumoxide_webkit_protocol::webkit::protocol as proto;

fn params<C: Command>(cmd: &C) -> Result<serde_json::Value> {
    serde_json::to_value(cmd).map_err(|e| Error::InvalidArgument(e.to_string()))
}

fn response<C: Command>(cmd: &C, value: serde_json::Value) -> Result<C::Response> {
    C::response_from_value(value).map_err(|e| Error::Protocol {
        method: cmd.identifier().to_string(),
        code: 0,
        message: format!("unexpected response: {e}"),
    })
}

/// A browser command (`page_proxy: None`) or a page-proxy command.
pub(crate) async fn send<C: Command>(
    session: &dyn WebKitSession,
    page_proxy: Option<&str>,
    cmd: C,
) -> Result<C::Response> {
    let value = session
        .call(page_proxy, &cmd.identifier(), params(&cmd)?)
        .await?;
    response(&cmd, value)
}

/// A command to a page target.
pub(crate) async fn send_target<C: Command>(
    session: &dyn WebKitSession,
    page_proxy: &str,
    target: &str,
    cmd: C,
) -> Result<C::Response> {
    let value = session
        .call_target(page_proxy, target, &cmd.identifier(), params(&cmd)?)
        .await?;
    response(&cmd, value)
}
