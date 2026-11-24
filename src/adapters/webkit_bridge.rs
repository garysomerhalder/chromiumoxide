//! A WebSocket server relaying Playwright's WebKit inspector protocol
//! between clients and one WebKit connection (`browser.bind()`): browser
//! and page-proxy commands go through the connection; a client's page
//! target commands (`Target.sendMessageToTarget`) are sent as target
//! commands and answered as `Target.dispatchMessageFromTarget`, so ids
//! never clash with the owner's; `Playwright.close` only ends that client;
//! every event is forwarded in WebKit's wire form.

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
use crate::ports::{BoundBridge, BridgeAddress, WebKitBridge, WebKitSession};

/// The production WebKit bridge (localhost WebSocket).
#[derive(Debug, Default, Clone, Copy)]
pub struct WsWebKitBridge;

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

/// A WebKit event in its wire form.
fn wire_event(e: &crate::ports::RawEvent) -> Value {
    match e
        .session_id
        .as_deref()
        .map(|s| s.split_once('/').unwrap_or((s, "")))
    {
        None => json!({"method": e.method, "params": e.params}),
        Some((proxy, "")) => json!({"method": e.method, "params": e.params, "pageProxyId": proxy}),
        Some((proxy, target)) => json!({
            "method": "Target.dispatchMessageFromTarget",
            "params": {"targetId": target,
                       "message": json!({"method": e.method, "params": e.params}).to_string()},
            "pageProxyId": proxy,
        }),
    }
}

fn error_json(e: Error) -> Value {
    match e {
        Error::Protocol { code, message, .. } => json!({"code": code, "message": message}),
        other => json!({"code": -32000, "message": other.to_string()}),
    }
}

async fn serve_client(wk: Arc<dyn WebKitSession>, stream: tokio::net::TcpStream, path: Arc<str>) {
    use async_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
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
    let mut events = wk.subscribe();
    let etx = tx.clone();
    let forward = tokio::spawn(async move {
        while let Some(e) = events.next().await {
            if etx.unbounded_send(wire_event(&e).to_string()).is_err() {
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
        let method = req["method"].as_str().unwrap_or_default().to_owned();
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
        let proxy = req["pageProxyId"].as_str().map(str::to_owned);
        let tx = tx.clone();
        if method == "Playwright.close" {
            let _ = tx.unbounded_send(json!({"id": id, "result": {}}).to_string());
            break;
        }
        if method == "Target.sendMessageToTarget"
            && let Some(proxy) = proxy.clone()
        {
            let target = params["targetId"].as_str().unwrap_or_default().to_owned();
            let inner: Value = params["message"]
                .as_str()
                .and_then(|m| serde_json::from_str(m).ok())
                .unwrap_or_default();
            let call = wk.call_target(
                &proxy,
                &target,
                inner["method"].as_str().unwrap_or_default(),
                inner.get("params").cloned().unwrap_or_else(|| json!({})),
            );
            let _ = tx
                .unbounded_send(json!({"id": id, "result": {}, "pageProxyId": proxy}).to_string());
            tokio::spawn(async move {
                let answer = match call.await {
                    Ok(result) => json!({"id": inner["id"], "result": result}),
                    Err(e) => json!({"id": inner["id"], "error": error_json(e)}),
                };
                let _ = tx.unbounded_send(
                    json!({"method": "Target.dispatchMessageFromTarget",
                           "params": {"targetId": target, "message": answer.to_string()},
                           "pageProxyId": proxy})
                    .to_string(),
                );
            });
            continue;
        }
        let call = wk.call(proxy.as_deref(), &method, params);
        tokio::spawn(async move {
            let mut reply = match call.await {
                Ok(result) => json!({"id": id, "result": result}),
                Err(e) => json!({"id": id, "error": error_json(e)}),
            };
            if let Some(p) = proxy {
                reply["pageProxyId"] = json!(p);
            }
            let _ = tx.unbounded_send(reply.to_string());
        });
    }
    forward.abort();
    writer.abort();
}

impl WebKitBridge for WsWebKitBridge {
    fn serve(
        &self,
        wk: Arc<dyn WebKitSession>,
        address: BridgeAddress,
    ) -> BoxFuture<'static, io::Result<Box<dyn BoundBridge>>> {
        async move {
            let host = address.host.unwrap_or_else(|| "127.0.0.1".into());
            let listener = TcpListener::bind((host.as_str(), address.port.unwrap_or(0))).await?;
            let port = listener.local_addr()?.port();
            let path: Arc<str> = match address.path {
                Some(p) if p.starts_with('/') => p.into(),
                Some(p) => format!("/{p}").into(),
                None => format!("/{}", crate::adapters::cdp::bridge::unguessable()).into(),
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
                    let client = tokio::spawn(serve_client(wk.clone(), stream, path.clone()));
                    t.lock().unwrap().push(client);
                }
            });
            tasks.lock().unwrap().push(accept);
            Ok(Box::new(Bound { endpoint, tasks }) as Box<dyn BoundBridge>)
        }
        .boxed()
    }
}
