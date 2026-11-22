//! [`CdpSession`] over any [`Transport`]: assigns ids, routes responses and
//! fans out events.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{Value, json};

use crate::domain::error::{Error, Result};
use crate::ports::{CdpSession, EventStream, RawEvent, Transport};

type Reply = oneshot::Sender<Result<Value>>;

struct Inner {
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, (String, Reply)>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Arc<RawEvent>>>>,
    writer: mpsc::UnboundedSender<String>,
    closed: AtomicBool,
}

/// A live CDP connection. Cheap to clone.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl Connection {
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
                    tracing::debug!("cdp write failed: {e}");
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
                        tracing::debug!("cdp read failed: {e}");
                        break;
                    }
                }
            }
            reader.close("connection closed");
        });

        Self { inner }
    }

    /// True once the transport closed.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

impl Inner {
    fn dispatch(&self, text: &str) {
        tracing::trace!(target: "chromiumoxide::cdp", "◀ {text}");
        let Ok(mut message) = serde_json::from_str::<Value>(text) else {
            tracing::debug!("unparseable cdp message: {text}");
            return;
        };
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            let Some((method, reply)) = self.pending.lock().unwrap().remove(&id) else {
                return;
            };
            let result = match message.get_mut("error") {
                Some(error) => Err(Error::Protocol {
                    method,
                    code: error
                        .get("code")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                }),
                None => Ok(message
                    .get_mut("result")
                    .map(Value::take)
                    .unwrap_or(json!({}))),
            };
            let _ = reply.send(result);
        } else if let Some(method) = message.get("method").and_then(Value::as_str) {
            let event = Arc::new(RawEvent {
                session_id: message
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                method: method.to_owned(),
                params: message
                    .get_mut("params")
                    .map(Value::take)
                    .unwrap_or(json!({})),
            });
            self.subscribers
                .lock()
                .unwrap()
                .retain(|s| s.unbounded_send(event.clone()).is_ok());
        }
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

impl CdpSession for Connection {
    fn disconnect(&self) {
        self.inner.writer.close_channel();
    }

    fn call(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> BoxFuture<'static, Result<Value>> {
        let closed =
            || futures::future::ready(Err(Error::TargetClosed("connection closed".into()))).boxed();
        if self.is_closed() {
            return closed();
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        let (tx, rx) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .unwrap()
            .insert(id, (method.to_owned(), tx));
        let text = message.to_string();
        tracing::trace!(target: "chromiumoxide::cdp", "▶ {text}");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::cdp::memory_transport::{Peer, pair};
    use crate::domain::error::Error;
    use futures::StreamExt;
    use serde_json::json;

    async fn next_sent(peer: &mut Peer) -> Value {
        serde_json::from_str(&peer.outgoing.next().await.expect("sent")).unwrap()
    }

    fn reply(peer: &Peer, v: Value) {
        peer.incoming.unbounded_send(Ok(v.to_string())).unwrap();
    }

    #[tokio::test]
    async fn call_sends_id_method_params_and_resolves_result() {
        let (t, mut peer) = pair();
        let conn = Connection::spawn(t);
        let fut = conn.call(None, "Browser.getVersion", json!({}));
        let task = tokio::spawn(fut);
        let sent = next_sent(&mut peer).await;
        assert_eq!(
            sent,
            json!({"id": 1, "method": "Browser.getVersion", "params": {}})
        );
        reply(
            &peer,
            json!({"id": 1, "result": {"product": "HeadlessChrome/157"}}),
        );
        assert_eq!(
            task.await.unwrap().unwrap(),
            json!({"product": "HeadlessChrome/157"})
        );
    }

    #[tokio::test]
    async fn session_id_is_sent_and_ids_increase() {
        let (t, mut peer) = pair();
        let conn = Connection::spawn(t);
        let a = tokio::spawn(conn.call(Some("S1"), "Page.enable", json!({})));
        let b = tokio::spawn(conn.call(Some("S1"), "Runtime.enable", json!({})));
        let first = next_sent(&mut peer).await;
        let second = next_sent(&mut peer).await;
        assert_eq!(first["sessionId"], "S1");
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        // out-of-order responses are matched by id
        reply(
            &peer,
            json!({"id": 2, "result": {"b": true}, "sessionId": "S1"}),
        );
        reply(
            &peer,
            json!({"id": 1, "result": {"a": true}, "sessionId": "S1"}),
        );
        assert_eq!(a.await.unwrap().unwrap(), json!({"a": true}));
        assert_eq!(b.await.unwrap().unwrap(), json!({"b": true}));
    }

    #[tokio::test]
    async fn error_response_becomes_protocol_error() {
        let (t, mut peer) = pair();
        let conn = Connection::spawn(t);
        let task = tokio::spawn(conn.call(None, "Nope.nothing", json!({})));
        let sent = next_sent(&mut peer).await;
        reply(
            &peer,
            json!({"id": sent["id"], "error": {"code": -32601, "message": "'Nope.nothing' wasn't found"}}),
        );
        match task.await.unwrap() {
            Err(Error::Protocol {
                method,
                code,
                message,
            }) => {
                assert_eq!(
                    (method.as_str(), code, message.as_str()),
                    ("Nope.nothing", -32601, "'Nope.nothing' wasn't found")
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn events_fan_out_to_every_subscriber_in_order() {
        let (t, peer) = pair();
        let conn = Connection::spawn(t);
        let mut s1 = conn.subscribe();
        let mut s2 = conn.subscribe();
        reply(
            &peer,
            json!({"method": "Target.attachedToTarget", "params": {"sessionId": "S1"}}),
        );
        reply(
            &peer,
            json!({"method": "Page.loadEventFired", "params": {"timestamp": 1.0}, "sessionId": "S1"}),
        );
        for s in [&mut s1, &mut s2] {
            let e = s.next().await.unwrap();
            assert_eq!(
                (e.method.as_str(), e.session_id.as_deref()),
                ("Target.attachedToTarget", None)
            );
            let e = s.next().await.unwrap();
            assert_eq!(
                (e.method.as_str(), e.session_id.as_deref()),
                ("Page.loadEventFired", Some("S1"))
            );
            assert_eq!(e.params, json!({"timestamp": 1.0}));
        }
    }

    #[tokio::test]
    async fn closing_the_transport_fails_pending_calls_and_ends_streams() {
        let (t, mut peer) = pair();
        let conn = Connection::spawn(t);
        let mut events = conn.subscribe();
        let task = tokio::spawn(conn.call(None, "Browser.getVersion", json!({})));
        let _ = next_sent(&mut peer).await;
        drop(peer);
        assert!(matches!(task.await.unwrap(), Err(Error::TargetClosed(_))));
        assert!(events.next().await.is_none());
        assert!(conn.is_closed());
        assert!(matches!(
            conn.call(None, "X.y", json!({})).await,
            Err(Error::TargetClosed(_))
        ));
    }
}
