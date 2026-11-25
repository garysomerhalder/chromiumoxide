//! A WebSocket server relaying WebDriver BiDi between clients and one
//! Firefox session (Firefox allows one session at a time): a client's
//! `session.new` is answered with the shared session, `session.end` only
//! ends that client, other commands are re-sent through the connection, and
//! every browser event is forwarded.

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
use crate::ports::{BidiBridge, BidiSession, BoundBridge, BridgeAddress};

/// The production BiDi bridge (localhost WebSocket).
#[derive(Debug, Default, Clone, Copy)]
pub struct WsBidiBridge;

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

async fn serve_client(
    bidi: Arc<dyn BidiSession>,
    stream: tokio::net::TcpStream,
    path: Arc<str>,
    capabilities: Value,
) {
    use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    // The Result type is tungstenite's handshake callback signature.
    #[allow(clippy::result_large_err)]
    let check =
        move |req: &Request, resp: Response| -> std::result::Result<Response, ErrorResponse> {
            // `Firefox::connect` adds `/session` to an endpoint without it.
            let asked = req.uri().path();
            if asked == &*path || asked.strip_suffix("/session") == Some(&*path) {
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
    let mut events = bidi.subscribe();
    let etx = tx.clone();
    let forward = tokio::spawn(async move {
        while let Some(e) = events.next().await {
            let m = json!({"type": "event", "method": e.method, "params": e.params});
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
        let local = match method.as_str() {
            "session.new" => Some(json!({"sessionId": "shared", "capabilities": capabilities})),
            "session.end" => Some(json!({})),
            _ => None,
        };
        let tx = tx.clone();
        if let Some(result) = local {
            let _ = tx
                .unbounded_send(json!({"type": "success", "id": id, "result": result}).to_string());
            if method == "session.end" {
                break;
            }
            continue;
        }
        let call = bidi.call(&method, params);
        tokio::spawn(async move {
            let reply = match call.await {
                Ok(result) => json!({"type": "success", "id": id, "result": result}),
                Err(Error::Protocol { message, .. }) => {
                    let (code, text) = message
                        .split_once(": ")
                        .map(|(c, t)| (c.to_owned(), t.to_owned()))
                        .unwrap_or_else(|| ("unknown error".into(), message.clone()));
                    json!({"type": "error", "id": id, "error": code, "message": text})
                }
                Err(e) => {
                    json!({"type": "error", "id": id, "error": "unknown error", "message": e.to_string()})
                }
            };
            let _ = tx.unbounded_send(reply.to_string());
        });
    }
    forward.abort();
    writer.abort();
}

impl BidiBridge for WsBidiBridge {
    fn serve(
        &self,
        bidi: Arc<dyn BidiSession>,
        address: BridgeAddress,
        capabilities: Value,
    ) -> BoxFuture<'static, io::Result<Box<dyn BoundBridge>>> {
        async move {
            let host = address.host.unwrap_or_else(|| "127.0.0.1".into());
            let listener = TcpListener::bind((host.as_str(), address.port.unwrap_or(0))).await?;
            let port = listener.local_addr()?.port();
            let path: Arc<str> = match address.path {
                Some(p) if p.starts_with('/') => p.into(),
                Some(p) => format!("/{p}").into(),
                None => format!("/{}/session", crate::adapters::cdp::bridge::unguessable()).into(),
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
                    let client = tokio::spawn(serve_client(
                        bidi.clone(),
                        stream,
                        path.clone(),
                        capabilities.clone(),
                    ));
                    t.lock().unwrap().push(client);
                }
            });
            tasks.lock().unwrap().push(accept);
            Ok(Box::new(Bound { endpoint, tasks }) as Box<dyn BoundBridge>)
        }
        .boxed()
    }
}
