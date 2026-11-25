//! Dedicated workers and WebSockets on the Firefox engine.
//!
//! Workers are BiDi `dedicated-worker` realms owned by one of the page's
//! window realms (Firefox 159 reports them but cannot run code in them,
//! so `evaluate` is an error there). BiDi has no WebSocket events, so a preload script wraps
//! the page's `WebSocket` (a `Proxy`, so it still looks native) and reports
//! each socket's frames in order over a `script.message` channel.

use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::firefox::Page;
use crate::domain::error::{Error, Result};

/// The `script.message` channel of the WebSocket preload script.
pub(crate) const WS_CHANNEL: &str = "__chromiumoxide_websocket";

/// Reports `open`, `sent`, `recv`, `error` and `close` per socket. Binary
/// frames are base64; Blob frames are read before later reports go out.
pub(crate) const WS_PRELOAD: &str = r#"(report) => {
  const Native = window.WebSocket;
  if (!Native || Native.__chromiumoxide) return;
  const ids = new WeakMap();
  let next = 1;
  let chain = Promise.resolve();
  const b64 = buf => {
    const bytes = new Uint8Array(buf);
    let s = '';
    for (let i = 0; i < bytes.length; i += 0x8000)
      s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(s);
  };
  const post = (msg, data) => {
    chain = chain.then(async () => {
      if (data !== undefined) {
        if (typeof data === 'string') msg.text = data;
        else if (data instanceof Blob) msg.b64 = b64(await data.arrayBuffer());
        else if (ArrayBuffer.isView(data)) msg.b64 = b64(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
        else msg.b64 = b64(data);
      }
      report(JSON.stringify(msg));
    }).catch(() => {});
  };
  const Wrapped = new Proxy(Native, {
    construct(target, args, newTarget) {
      const ws = Reflect.construct(target, args, newTarget === Wrapped ? target : newTarget);
      const id = next++;
      ids.set(ws, id);
      post({ type: 'open', id, url: ws.url });
      ws.addEventListener('message', e => post({ type: 'recv', id }, e.data));
      ws.addEventListener('error', () => post({ type: 'error', id, message: 'WebSocket error' }));
      ws.addEventListener('close', () => post({ type: 'close', id }));
      return ws;
    },
    get(target, key, receiver) {
      return key === '__chromiumoxide' ? true : Reflect.get(target, key, receiver);
    },
  });
  const send = Native.prototype.send;
  Native.prototype.send = new Proxy(send, {
    apply(target, self, args) {
      const id = ids.get(self);
      const r = Reflect.apply(target, self, args);
      if (id !== undefined) post({ type: 'sent', id }, args[0]);
      return r;
    },
  });
  window.WebSocket = Wrapped;
}"#;

/// A dedicated worker of a Firefox page (`page.workers()`). Cheap to clone.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<WorkerInner>,
}

/// `worker.on(...)` events.
#[derive(Clone, Debug)]
pub enum WorkerEvent {
    /// The worker was terminated (or its page went away).
    Close,
}

/// Firefox finds script targets among window realms only.
fn worker_realm_error(e: Error) -> Error {
    match e {
        Error::Protocol { message, .. } if message.contains("no such frame") => {
            Error::InvalidArgument(format!(
                "worker.evaluate is not supported on Firefox: its WebDriver BiDi cannot run \
                 scripts in worker realms yet ({message})"
            ))
        }
        other => other,
    }
}

struct WorkerInner {
    page: Page,
    realm: String,
    url: String,
    closed: watch::Sender<bool>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Worker")
            .field("url", &self.inner.url)
            .finish()
    }
}

impl Worker {
    pub(crate) fn new(page: Page, realm: String, url: String) -> Self {
        let (closed, _) = watch::channel(false);
        Worker {
            inner: Arc::new(WorkerInner {
                page,
                realm,
                url,
                closed,
            }),
        }
    }

    pub(crate) fn realm(&self) -> &str {
        &self.inner.realm
    }

    pub(crate) fn mark_closed(&self) {
        self.inner.closed.send_replace(true);
    }

    /// The worker script's URL.
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn is_closed(&self) -> bool {
        *self.inner.closed.borrow()
    }

    /// Waits until the worker is terminated (or its page goes away).
    pub async fn wait_for_close(&self) {
        let mut rx = self.inner.closed.subscribe();
        let _ = rx.wait_for(|closed| *closed).await;
    }

    async fn evaluate_value(&self, source: &str, arg: Option<&Value>) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::TargetClosed("worker has been closed".into()));
        }
        self.inner
            .page
            .evaluate_value_in(json!({"realm": self.inner.realm}), source, arg)
            .await
            .map_err(worker_realm_error)
    }

    /// `worker.evaluateHandle(expression or function)`.
    pub async fn evaluate_handle(
        &self,
        source: &str,
    ) -> Result<crate::application::firefox_handle::JsHandle> {
        if self.is_closed() {
            return Err(Error::TargetClosed("worker has been closed".into()));
        }
        self.inner
            .page
            .evaluate_handle_in(json!({"realm": self.inner.realm}), source, None)
            .await
            .map_err(worker_realm_error)
    }

    /// `worker.waitForEvent(event)`: the first event `matcher` maps to a
    /// value (Firefox reports only the worker's close).
    pub async fn wait_for_event<T>(
        &self,
        matcher: impl Fn(&WorkerEvent) -> Option<T>,
    ) -> Result<T> {
        self.wait_for_close().await;
        matcher(&WorkerEvent::Close)
            .ok_or_else(|| Error::TargetClosed("worker has been closed".into()))
    }

    /// `worker.evaluate(expression or function)`.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.evaluate_value(source, None).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `worker.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self.evaluate_value(source, Some(&arg)).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }
}
