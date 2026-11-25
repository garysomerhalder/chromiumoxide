//! A scripted [`CdpSession`] for application tests.

use std::sync::{Arc, Mutex};

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde_json::{Value, json};

use crate::domain::error::Result;
use crate::ports::{CdpSession, EventStream, RawEvent};

/// A recorded call.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

type Handler = dyn Fn(&FakeCdp, &Call) -> Result<Value> + Send + Sync;

/// Answers calls with a handler (default: `{}`) and records them.
#[derive(Clone, Default)]
pub struct FakeCdp {
    calls: Arc<Mutex<Vec<Call>>>,
    subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<Arc<RawEvent>>>>>,
    handler: Arc<Mutex<Option<Arc<Handler>>>>,
}

impl FakeCdp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the response handler.
    pub fn on_call(&self, f: impl Fn(&FakeCdp, &Call) -> Result<Value> + Send + Sync + 'static) {
        *self.handler.lock().unwrap() = Some(Arc::new(f));
    }

    /// Emits an event to all subscribers.
    pub fn emit(&self, session_id: Option<&str>, method: &str, params: Value) {
        let e = Arc::new(RawEvent {
            session_id: session_id.map(str::to_owned),
            method: method.to_owned(),
            params,
        });
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(e.clone()).is_ok());
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn methods(&self) -> Vec<String> {
        self.calls().into_iter().map(|c| c.method).collect()
    }

    /// Calls to `method`.
    pub fn calls_to(&self, method: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.method == method)
            .collect()
    }
}

impl CdpSession for FakeCdp {
    fn call(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>> {
        let call = Call {
            session_id: session_id.map(str::to_owned),
            method: method.to_owned(),
            params,
        };
        self.calls.lock().unwrap().push(call.clone());
        let handler = self.handler.lock().unwrap().clone();
        let result = match handler {
            Some(h) => h(self, &call),
            None => Ok(json!({})),
        };
        futures::future::ready(result).boxed()
    }

    fn subscribe(&self) -> EventStream {
        let (tx, rx) = mpsc::unbounded();
        self.subscribers.lock().unwrap().push(tx);
        rx.boxed()
    }
}

/// The same fake as a WebDriver BiDi connection (calls have no session).
impl crate::ports::BidiSession for FakeCdp {
    fn call(&self, method: &str, params: Value) -> BoxFuture<'static, Result<Value>> {
        CdpSession::call(self, None, method, params)
    }

    fn subscribe(&self) -> EventStream {
        CdpSession::subscribe(self)
    }
}
