//! [`WebKitSession`] over any [`Transport`] (Playwright's WebKit
//! `--inspector-pipe`): ids, page-proxy routing (`pageProxyId`) and the
//! page-target layer (`Target.sendMessageToTarget` /
//! `Target.dispatchMessageFromTarget`), like Playwright's `WKConnection`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{Value, json};

use crate::domain::error::{Error, Result};
use crate::ports::webkit::WebKitSession;
use crate::ports::{EventStream, RawEvent, Transport};

type Reply = oneshot::Sender<Result<Value>>;

enum Pending {
    /// A browser or page-proxy command.
    Call(String, Reply),
    /// The `Target.sendMessageToTarget` carrying the target command `inner`.
    Carrier { inner: u64 },
    /// A target command; its reply comes in `Target.dispatchMessageFromTarget`.
    Target(String, Reply),
}

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Arc<RawEvent>>>>,
    writer: mpsc::UnboundedSender<String>,
    closed: AtomicBool,
}

/// A live connection to Playwright's WebKit. Cheap to clone.
#[derive(Clone)]
pub struct WebKitConnection {
    inner: Arc<Inner>,
}

fn protocol_error(method: String, error: &Value) -> Error {
    Error::Protocol {
        method,
        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

impl WebKitConnection {
    /// Starts the connection's I/O tasks on the current tokio runtime.
    pub fn spawn(transport: Transport) -> Self {
        let Transport {
            mut sink,
            mut stream,
        } = transport;
        let (writer, mut outgoing) = mpsc::unbounded::<String>();
        let inner = Arc::new(Inner {
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            writer,
            closed: AtomicBool::new(false),
        });
        tokio::spawn(async move {
            while let Some(message) = outgoing.next().await {
                if let Err(e) = sink.send(message).await {
                    tracing::debug!("webkit write failed: {e}");
                    break;
                }
            }
        });
        let reader = inner.clone();
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                match message {
                    Ok(text) => reader.dispatch(&text),
                    Err(e) => {
                        tracing::debug!("webkit read failed: {e}");
                        break;
                    }
                }
            }
            reader.close("connection closed");
        });
        Self { inner }
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    fn send(&self, id: u64, pending: Pending, message: Value) -> bool {
        self.inner.pending.lock().unwrap().insert(id, pending);
        let text = message.to_string();
        tracing::trace!(target: "chromiumoxide::webkit", "▶ {text}");
        if self.inner.writer.unbounded_send(text).is_err() || self.is_closed() {
            self.inner.pending.lock().unwrap().remove(&id);
            return false;
        }
        true
    }
}

fn closed() -> BoxFuture<'static, Result<Value>> {
    futures::future::ready(Err(Error::TargetClosed("connection closed".into()))).boxed()
}

impl Inner {
    fn emit(&self, session_id: Option<String>, method: String, params: Value) {
        let event = Arc::new(RawEvent {
            session_id,
            method,
            params,
        });
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }

    fn dispatch(&self, text: &str) {
        tracing::trace!(target: "chromiumoxide::webkit", "◀ {text}");
        let Ok(mut message) = serde_json::from_str::<Value>(text) else {
            tracing::debug!("unparseable webkit message: {text}");
            return;
        };
        let proxy = message
            .get("pageProxyId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            let Some(pending) = self.pending.lock().unwrap().remove(&id) else {
                return;
            };
            match pending {
                Pending::Call(method, reply) => {
                    let result = match message.get("error") {
                        Some(error) => Err(protocol_error(method, error)),
                        None => Ok(message
                            .get_mut("result")
                            .map(Value::take)
                            .unwrap_or(json!({}))),
                    };
                    let _ = reply.send(result);
                }
                // The target is gone: fail the command it carried.
                Pending::Carrier { inner } => {
                    if let Some(error) = message.get("error")
                        && let Some(Pending::Target(method, reply)) =
                            self.pending.lock().unwrap().remove(&inner)
                    {
                        let _ = reply.send(Err(protocol_error(method, error)));
                    }
                }
                Pending::Target(..) => {}
            }
            return;
        }
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut params = message
            .get_mut("params")
            .map(Value::take)
            .unwrap_or(json!({}));
        if method == "Target.dispatchMessageFromTarget"
            && let Some(proxy) = &proxy
        {
            let target = params["targetId"].as_str().unwrap_or_default().to_owned();
            let Some(mut inner) = params["message"]
                .as_str()
                .and_then(|m| serde_json::from_str::<Value>(m).ok())
            else {
                return;
            };
            if let Some(id) = inner.get("id").and_then(Value::as_u64) {
                if let Some(Pending::Target(method, reply)) =
                    self.pending.lock().unwrap().remove(&id)
                {
                    let result = match inner.get("error") {
                        Some(error) => Err(protocol_error(method, error)),
                        None => Ok(inner
                            .get_mut("result")
                            .map(Value::take)
                            .unwrap_or(json!({}))),
                    };
                    let _ = reply.send(result);
                }
                return;
            }
            let method = inner["method"].as_str().unwrap_or_default().to_owned();
            let params = inner
                .get_mut("params")
                .map(Value::take)
                .unwrap_or(json!({}));
            self.emit(Some(format!("{proxy}/{target}")), method, params);
            return;
        }
        self.emit(proxy, method, std::mem::take(&mut params));
    }

    fn close(&self, reason: &str) {
        self.closed.store(true, Ordering::SeqCst);
        let pending: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (_, p) in pending {
            if let Pending::Call(_, reply) | Pending::Target(_, reply) = p {
                let _ = reply.send(Err(Error::TargetClosed(reason.to_owned())));
            }
        }
        self.subscribers.lock().unwrap().clear();
        self.writer.close_channel();
    }
}

fn wait(rx: oneshot::Receiver<Result<Value>>) -> BoxFuture<'static, Result<Value>> {
    async move {
        rx.await
            .unwrap_or_else(|_| Err(Error::TargetClosed("connection closed".into())))
    }
    .boxed()
}

impl WebKitSession for WebKitConnection {
    fn call(
        &self,
        page_proxy: Option<&str>,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>> {
        if self.is_closed() {
            return closed();
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(proxy) = page_proxy {
            message["pageProxyId"] = json!(proxy);
        }
        let (tx, rx) = oneshot::channel();
        if !self.send(id, Pending::Call(method.to_owned(), tx), message) {
            return closed();
        }
        wait(rx)
    }

    fn call_target(
        &self,
        page_proxy: &str,
        target: &str,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>> {
        if self.is_closed() {
            return closed();
        }
        let inner = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .unwrap()
            .insert(inner, Pending::Target(method.to_owned(), tx));
        let carrier = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let message = json!({
            "id": carrier,
            "method": "Target.sendMessageToTarget",
            "params": {
                "targetId": target,
                "message": json!({"id": inner, "method": method, "params": params}).to_string(),
            },
            "pageProxyId": page_proxy,
        });
        if !self.send(carrier, Pending::Carrier { inner }, message) {
            self.inner.pending.lock().unwrap().remove(&inner);
            return closed();
        }
        wait(rx)
    }

    fn subscribe(&self) -> EventStream {
        let (tx, rx) = mpsc::unbounded();
        if !self.is_closed() {
            self.inner.subscribers.lock().unwrap().push(tx);
        }
        rx.boxed()
    }

    fn disconnect(&self) {
        self.inner.writer.close_channel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cdp::memory_transport::pair;

    #[tokio::test]
    async fn routes_browser_proxy_and_target_messages() {
        let (transport, mut peer) = pair();
        let wk = WebKitConnection::spawn(transport);
        let mut events = wk.subscribe();
        let next = |s: Option<String>| -> Value { serde_json::from_str(&s.unwrap()).unwrap() };

        // A browser command.
        let enable = wk.call(None, "Playwright.enable", json!({}));
        let sent = next(peer.outgoing.next().await);
        assert_eq!(sent["method"], "Playwright.enable");
        assert!(sent.get("pageProxyId").is_none());
        peer.incoming
            .unbounded_send(Ok(json!({"id": sent["id"], "result": {}}).to_string()))
            .unwrap();
        assert_eq!(enable.await.unwrap(), json!({}));

        // A page-proxy command carries pageProxyId; errors become Protocol.
        let metrics = wk.call(Some("5"), "Emulation.setDeviceMetricsOverride", json!({}));
        let sent = next(peer.outgoing.next().await);
        assert_eq!(sent["pageProxyId"], "5");
        peer.incoming
            .unbounded_send(Ok(json!({"id": sent["id"], "error": {"code": -32000, "message": "bad"}, "pageProxyId": "5"}).to_string()))
            .unwrap();
        let err = metrics.await.unwrap_err().to_string();
        assert!(err.contains("bad"), "{err}");

        // A target command goes inside Target.sendMessageToTarget; its reply
        // comes back in Target.dispatchMessageFromTarget.
        let eval = wk.call_target(
            "5",
            "page-6",
            "Runtime.evaluate",
            json!({"expression": "1"}),
        );
        let sent = next(peer.outgoing.next().await);
        assert_eq!(sent["method"], "Target.sendMessageToTarget");
        assert_eq!(sent["pageProxyId"], "5");
        assert_eq!(sent["params"]["targetId"], "page-6");
        let inner: Value =
            serde_json::from_str(sent["params"]["message"].as_str().unwrap()).unwrap();
        assert_eq!(inner["method"], "Runtime.evaluate");
        peer.incoming
            .unbounded_send(Ok(
                json!({"id": sent["id"], "result": {}, "pageProxyId": "5"}).to_string(),
            ))
            .unwrap();
        let reply =
            json!({"id": inner["id"], "result": {"result": {"type": "number", "value": 1}}})
                .to_string();
        peer.incoming
            .unbounded_send(Ok(json!({"method": "Target.dispatchMessageFromTarget",
                "params": {"targetId": "page-6", "message": reply}, "pageProxyId": "5"})
            .to_string()))
            .unwrap();
        assert_eq!(eval.await.unwrap()["result"]["value"], 1);

        // Events: browser, page proxy and page target.
        for (event, session) in [
            (
                json!({"method": "Playwright.pageProxyCreated", "params": {"pageProxyId": "5"}}),
                None,
            ),
            (
                json!({"method": "Target.targetCreated", "params": {"targetInfo": {}}, "pageProxyId": "5"}),
                Some("5"),
            ),
            (
                json!({"method": "Target.dispatchMessageFromTarget", "pageProxyId": "5",
                    "params": {"targetId": "page-6", "message": json!({"method": "Page.loadEventFired", "params": {"timestamp": 1}}).to_string()}}),
                Some("5/page-6"),
            ),
        ] {
            peer.incoming.unbounded_send(Ok(event.to_string())).unwrap();
            let got = events.next().await.unwrap();
            assert_eq!(got.session_id.as_deref(), session);
        }
    }
}
