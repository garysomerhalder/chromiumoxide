//! [`BidiSession`] over any [`Transport`]: assigns ids, routes responses and
//! fans out events (`{"type": "success" | "error" | "event", ...}`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{Value, json};

use crate::domain::error::{Error, Result};
use crate::ports::{BidiSession, EventStream, RawEvent, Transport};

type Reply = oneshot::Sender<Result<Value>>;

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, (String, Reply)>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Arc<RawEvent>>>>,
    writer: mpsc::UnboundedSender<String>,
    closed: AtomicBool,
}

/// A live BiDi connection. Cheap to clone.
#[derive(Clone)]
pub struct BidiConnection {
    inner: Arc<Inner>,
}

impl BidiConnection {
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
                    tracing::debug!("bidi write failed: {e}");
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
                        tracing::debug!("bidi read failed: {e}");
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
}

impl Inner {
    fn dispatch(&self, text: &str) {
        tracing::trace!(target: "chromiumoxide::bidi", "◀ {text}");
        let Ok(mut message) = serde_json::from_str::<Value>(text) else {
            tracing::debug!("unparseable bidi message: {text}");
            return;
        };
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "event" {
            let method = message
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let event = Arc::new(RawEvent {
                session_id: None,
                method,
                params: message
                    .get_mut("params")
                    .map(Value::take)
                    .unwrap_or(json!({})),
            });
            self.subscribers
                .lock()
                .unwrap()
                .retain(|s| s.unbounded_send(event.clone()).is_ok());
            return;
        }
        let Some(id) = message.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some((method, reply)) = self.pending.lock().unwrap().remove(&id) else {
            return;
        };
        let result = if kind == "error" {
            let error = message.get("error").and_then(Value::as_str).unwrap_or("");
            let text = message.get("message").and_then(Value::as_str).unwrap_or("");
            Err(Error::Protocol {
                method,
                code: 0,
                message: format!("{error}: {text}"),
            })
        } else {
            Ok(message
                .get_mut("result")
                .map(Value::take)
                .unwrap_or(json!({})))
        };
        let _ = reply.send(result);
    }

    fn close(&self, reason: &str) {
        self.closed.store(true, Ordering::SeqCst);
        let pending: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (_, (_, reply)) in pending {
            let _ = reply.send(Err(Error::TargetClosed(reason.to_owned())));
        }
        self.subscribers.lock().unwrap().clear();
        self.writer.close_channel();
    }
}

impl BidiSession for BidiConnection {
    fn call(&self, method: &str, params: Value) -> BoxFuture<'static, Result<Value>> {
        let closed =
            || futures::future::ready(Err(Error::TargetClosed("connection closed".into()))).boxed();
        if self.is_closed() {
            return closed();
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let text = json!({ "id": id, "method": method, "params": params }).to_string();
        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .unwrap()
            .insert(id, (method.to_owned(), tx));
        tracing::trace!(target: "chromiumoxide::bidi", "▶ {text}");
        if self.inner.writer.unbounded_send(text).is_err() || self.is_closed() {
            self.inner.pending.lock().unwrap().remove(&id);
            return closed();
        }
        async move {
            rx.await
                .unwrap_or_else(|_| Err(Error::TargetClosed("connection closed".into())))
        }
        .boxed()
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
    async fn routes_results_errors_and_events() {
        let (transport, mut peer) = pair();
        let bidi = BidiConnection::spawn(transport);
        let mut events = bidi.subscribe();

        let ok = bidi.call("session.status", json!({}));
        let sent: Value = serde_json::from_str(&peer.outgoing.next().await.unwrap()).unwrap();
        assert_eq!(sent["method"], "session.status");
        peer.incoming
            .unbounded_send(Ok(
                json!({"type": "success", "id": sent["id"], "result": {"ready": true}}).to_string(),
            ))
            .unwrap();
        assert_eq!(ok.await.unwrap(), json!({"ready": true}));

        let bad = bidi.call("browsingContext.navigate", json!({}));
        let sent: Value = serde_json::from_str(&peer.outgoing.next().await.unwrap()).unwrap();
        peer.incoming
            .unbounded_send(Ok(json!({"type": "error", "id": sent["id"], "error": "no such frame", "message": "gone"}).to_string()))
            .unwrap();
        match bad.await {
            Err(Error::Protocol {
                method, message, ..
            }) => {
                assert_eq!(method, "browsingContext.navigate");
                assert_eq!(message, "no such frame: gone");
            }
            other => panic!("{other:?}"),
        }

        peer.incoming
            .unbounded_send(Ok(
                json!({"type": "event", "method": "log.entryAdded", "params": {"text": "hi"}})
                    .to_string(),
            ))
            .unwrap();
        let event = events.next().await.unwrap();
        assert_eq!(event.method, "log.entryAdded");
        assert_eq!(event.params["text"], "hi");
    }
}
