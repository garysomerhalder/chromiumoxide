//! WebSockets opened by a page (Playwright's `WebSocket`).

use std::sync::{Arc, Mutex};

use base64::Engine;
use futures::channel::mpsc;
use futures::{Stream, StreamExt};
use serde_json::Value;

use crate::application::events::{EventWaiter, PageEvent};
use crate::application::page::Page;
use crate::ports::RawEvent;

/// A WebSocket message payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsPayload {
    Text(String),
    Binary(Vec<u8>),
}

/// Something that happened on a WebSocket.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum WebSocketEvent {
    /// `framesent`.
    FrameSent(WsPayload),
    /// `framereceived`.
    FrameReceived(WsPayload),
    /// `socketerror`.
    Error(String),
    /// `close`.
    Close,
}

/// A WebSocket of a page. Cheap to clone.
#[derive(Clone)]
pub struct WebSocket {
    inner: Arc<WsInner>,
}

struct WsInner {
    url: String,
    state: Mutex<WsState>,
}

#[derive(Default)]
struct WsState {
    history: Vec<WebSocketEvent>,
    subscribers: Vec<mpsc::UnboundedSender<WebSocketEvent>>,
    closed: bool,
}

impl std::fmt::Debug for WebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket")
            .field("url", &self.inner.url)
            .finish()
    }
}

impl WebSocket {
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn is_closed(&self) -> bool {
        self.inner.state.lock().unwrap().closed
    }

    /// All events of this socket: the ones so far, then new ones; the
    /// stream ends after `Close`.
    pub fn events(&self) -> impl Stream<Item = WebSocketEvent> + Send + use<> {
        let (tx, rx) = mpsc::unbounded();
        let mut st = self.inner.state.lock().unwrap();
        for e in &st.history {
            let _ = tx.unbounded_send(e.clone());
        }
        if !st.closed {
            st.subscribers.push(tx);
        }
        rx
    }

    /// Waits until the socket is closed.
    pub async fn wait_for_close(&self) {
        let mut events = self.events();
        while let Some(e) = events.next().await {
            if e == WebSocketEvent::Close {
                break;
            }
        }
    }

    /// A socket opened at `url` (for engines that report sockets themselves).
    pub(crate) fn new(url: String) -> Self {
        WebSocket {
            inner: Arc::new(WsInner {
                url,
                state: Mutex::new(WsState::default()),
            }),
        }
    }

    pub(crate) fn push(&self, event: WebSocketEvent) {
        let mut st = self.inner.state.lock().unwrap();
        if st.closed {
            return;
        }
        if event == WebSocketEvent::Close {
            st.closed = true;
        }
        st.history.push(event.clone());
        st.subscribers
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
        if st.closed {
            st.subscribers.clear();
        }
    }
}

pub(crate) fn payload(p: &Value) -> WsPayload {
    let data = p
        .pointer("/response/payloadData")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if p.pointer("/response/opcode").and_then(Value::as_u64) == Some(2) {
        WsPayload::Binary(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap_or_default(),
        )
    } else {
        WsPayload::Text(data.to_owned())
    }
}

impl Page {
    /// `Network.webSocket*` events.
    pub(crate) fn handle_websocket_event(&self, e: &RawEvent) {
        let p = &e.params;
        let id = p
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if e.method == "Network.webSocketCreated" {
            let ws = WebSocket {
                inner: Arc::new(WsInner {
                    url: p
                        .get("url")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    state: Mutex::new(WsState::default()),
                }),
            };
            self.inner.websockets.lock().unwrap().insert(id, ws.clone());
            self.emit(PageEvent::WebSocket(ws));
            return;
        }
        let ws = {
            let mut map = self.inner.websockets.lock().unwrap();
            if e.method == "Network.webSocketClosed" {
                map.remove(&id)
            } else {
                map.get(&id).cloned()
            }
        };
        let Some(ws) = ws else {
            return;
        };
        match e.method.as_str() {
            "Network.webSocketFrameSent" => ws.push(WebSocketEvent::FrameSent(payload(p))),
            "Network.webSocketFrameReceived" => ws.push(WebSocketEvent::FrameReceived(payload(p))),
            "Network.webSocketFrameError" => ws.push(WebSocketEvent::Error(
                p.get("errorMessage")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )),
            "Network.webSocketClosed" => ws.push(WebSocketEvent::Close),
            _ => {}
        }
    }

    /// Closes the page's sockets when the page goes away.
    pub(crate) fn close_websockets(&self) {
        let sockets: Vec<WebSocket> = self
            .inner
            .websockets
            .lock()
            .unwrap()
            .drain()
            .map(|(_, ws)| ws)
            .collect();
        for ws in sockets {
            ws.push(WebSocketEvent::Close);
        }
    }

    /// `page.waitForEvent('websocket')`.
    pub fn wait_for_websocket(&self) -> EventWaiter<'_, WebSocket> {
        self.wait_for_event(|e| match e {
            PageEvent::WebSocket(ws) => Some(ws.clone()),
            _ => None,
        })
    }
}
