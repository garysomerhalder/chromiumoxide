//! Typed escape hatch: any generated command or event, on a page or the browser.

use std::sync::Arc;

use chromiumoxide_types::{Command, MethodType};
use futures::{Stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::browser::Browser;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::ports::{CdpSession, RawEvent};

async fn execute<C: Command>(
    cdp: &dyn CdpSession,
    session: Option<&str>,
    cmd: C,
) -> Result<C::Response> {
    let method = cmd.identifier();
    let params = serde_json::to_value(&cmd)?;
    let params = if params.is_null() {
        Value::Object(Default::default())
    } else {
        params
    };
    let response = cdp.call(session, &method, params).await?;
    C::response_from_value(response).map_err(|e| Error::Serde(format!("{method} response: {e}")))
}

fn events<E>(
    cdp: &Arc<dyn CdpSession>,
    session: Option<String>,
) -> impl Stream<Item = E> + Send + use<E>
where
    E: MethodType + DeserializeOwned + Send + 'static,
{
    let method = E::method_id();
    cdp.subscribe().filter_map(move |e: Arc<RawEvent>| {
        let wanted = e.method == method && e.session_id == session;
        let parsed = wanted.then(|| serde_json::from_value::<E>(e.params.clone()));
        futures::future::ready(match parsed {
            Some(Ok(event)) => Some(event),
            Some(Err(err)) => {
                tracing::debug!("cannot parse {}: {err}", e.method);
                None
            }
            None => None,
        })
    })
}

impl Page {
    /// Sends any generated protocol command to this page's session.
    pub async fn execute<C: Command>(&self, cmd: C) -> Result<C::Response> {
        if self.is_closed() {
            return Err(Error::TargetClosed("page has been closed".into()));
        }
        execute(&*self.inner.shared.cdp, self.sid(), cmd).await
    }

    /// Stream of a generated event type from this page's session, from now on.
    pub fn cdp_events<E>(&self) -> impl Stream<Item = E> + Send + use<E>
    where
        E: MethodType + DeserializeOwned + Send + 'static,
    {
        events(&self.inner.shared.cdp, Some(self.inner.session_id.clone()))
    }
}

impl Browser {
    /// Sends any generated protocol command to the browser target.
    pub async fn execute<C: Command>(&self, cmd: C) -> Result<C::Response> {
        execute(&*self.cdp(), None, cmd).await
    }

    /// Stream of a generated event type from the browser target, from now on.
    pub fn cdp_events<E>(&self) -> impl Stream<Item = E> + Send + use<E>
    where
        E: MethodType + DeserializeOwned + Send + 'static,
    {
        events(&self.cdp(), None)
    }
}
