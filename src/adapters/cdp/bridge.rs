//! A WebSocket server relaying CDP between clients and one browser
//! connection: client commands are re-sent through the connection (so ids
//! never clash) and every browser event is forwarded.

use std::io;
use std::sync::{Arc, Mutex};

use async_tungstenite::tungstenite::Message;
use async_tungstenite::tungstenite::protocol::WebSocketConfig;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use crate::domain::error::Error;
use crate::ports::{BoundBridge, BridgeAddress, CdpBridge, CdpSession};

/// The production bridge (localhost WebSocket).
#[derive(Debug, Default, Clone, Copy)]
pub struct WsBridge;

struct Bound {
    endpoint: String,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BoundBridge for Bound {
    fn endpoint(&self) -> String {
        self.endpoint.clone()
    }
    fn stop(&self) {
        for t in self.tasks.lock().unwrap().drain(..) {
            t.abort();
        }
    }
}

impl Drop for Bound {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn serve_client(cdp: Arc<dyn CdpSession>, stream: tokio::net::TcpStream, path: Arc<str>) {
    use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    // Only the served path (Playwright's `wsPath`) is accepted.
    // The Result type is tungstenite's handshake callback signature.
    #[allow(clippy::result_large_err)]
    let check =
        move |req: &Request, resp: Response| -> std::result::Result<Response, ErrorResponse> {
            if req.uri().path() == &*path {
                Ok(resp)
            } else {
                let mut refused = ErrorResponse::new(Some("not found".into()));
                *refused.status_mut() = async_tungstenite::tungstenite::http::StatusCode::NOT_FOUND;
                Err(refused)
            }
        };
    let Ok(ws) =
        async_tungstenite::tokio::accept_hdr_async_with_config(stream, check, Some(config)).await
    else {
        return;
    };
    let (mut sink, mut incoming) = ws.split();
    let (tx, mut rx) = mpsc::unbounded::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = rx.next().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
    });
    let mut events = cdp.subscribe();
    let etx = tx.clone();
    let forward = tokio::spawn(async move {
        while let Some(e) = events.next().await {
            let mut m = json!({"method": e.method, "params": e.params});
            if let Some(s) = &e.session_id {
                m["sessionId"] = json!(s);
            }
            if etx.unbounded_send(m.to_string()).is_err() {
                break;
            }
        }
    });
    while let Some(Ok(msg)) = incoming.next().await {
        let text = match msg {
            Message::Text(t) => t.as_str().to_owned(),
            Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(req) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
        let session = req
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let call = cdp.call(session.as_deref(), &method, params);
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reply = match call.await {
                Ok(result) => json!({"id": id, "result": result}),
                Err(Error::Protocol { code, message, .. }) => {
                    json!({"id": id, "error": {"code": code, "message": message}})
                }
                Err(e) => json!({"id": id, "error": {"code": -32000, "message": e.to_string()}}),
            };
            if let Some(s) = session {
                reply["sessionId"] = json!(s);
            }
            let _ = tx.unbounded_send(reply.to_string());
        });
    }
    forward.abort();
    writer.abort();
}

impl CdpBridge for WsBridge {
    fn serve(
        &self,
        cdp: Arc<dyn CdpSession>,
        address: BridgeAddress,
    ) -> BoxFuture<'static, io::Result<Box<dyn BoundBridge>>> {
        async move {
            let host = address.host.unwrap_or_else(|| "127.0.0.1".into());
            let listener = TcpListener::bind((host.as_str(), address.port.unwrap_or(0))).await?;
            let port = listener.local_addr()?.port();
            let path: Arc<str> = match address.path {
                Some(p) if p.starts_with('/') => p.into(),
                Some(p) => format!("/{p}").into(),
                None => format!("/devtools/browser/{}", unguessable()).into(),
            };
            let endpoint_host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.clone()
            };
            let endpoint = format!("ws://{endpoint_host}:{port}{path}");
            let tasks: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
            let t = tasks.clone();
            let accept = tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    let client = tokio::spawn(serve_client(cdp.clone(), stream, path.clone()));
                    t.lock().unwrap().push(client);
                }
            });
            tasks.lock().unwrap().push(accept);
            Ok(Box::new(Bound { endpoint, tasks }) as Box<dyn BoundBridge>)
        }
        .boxed()
    }

    fn is_listening(&self, endpoint: &str) -> BoxFuture<'static, bool> {
        let addr = url::Url::parse(endpoint).ok().and_then(|u| {
            let host = u.host_str()?.trim_matches(['[', ']']).to_owned();
            Some((host, u.port_or_known_default()?))
        });
        async move {
            let Some((host, port)) = addr else {
                return false;
            };
            let connect = tokio::net::TcpStream::connect((host.as_str(), port));
            matches!(
                tokio::time::timeout(std::time::Duration::from_secs(2), connect).await,
                Ok(Ok(_))
            )
        }
        .boxed()
    }
}

/// 128 random bits as hex, from the OS-seeded hasher keys.
pub(crate) fn unguessable() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(std::process::id() as u64);
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}
