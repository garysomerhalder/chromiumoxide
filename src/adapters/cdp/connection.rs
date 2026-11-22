//! [`CdpSession`] over any [`Transport`]: assigns ids, routes responses and
//! fans out events.

use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::Value;

use crate::domain::error::Result;
use crate::ports::{CdpSession, EventStream, Transport};

/// A live CDP connection. Cheap to clone.
#[derive(Clone)]
pub struct Connection {
    _inner: Arc<()>,
}

impl Connection {
    /// Starts the connection's I/O tasks on the current tokio runtime.
    pub fn spawn(_transport: Transport) -> Self {
        unimplemented!()
    }

    /// True once the transport closed.
    pub fn is_closed(&self) -> bool {
        unimplemented!()
    }
}

impl CdpSession for Connection {
    fn call(
        &self,
        _session_id: Option<&str>,
        _method: &str,
        _params: Value,
    ) -> BoxFuture<'static, Result<Value>> {
        unimplemented!()
    }

    fn subscribe(&self) -> EventStream {
        unimplemented!()
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
