//! WebSocket routing (Playwright's `routeWebSocket` / `WebSocketRoute`):
//! Playwright's WebSocket mock (vendored, Apache-2.0) replaces the page's
//! `WebSocket` and reports to a binding; handlers answer through
//! `__pwWebSocketDispatch`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::page::Page;
use crate::application::scripts::BindingSource;
use crate::application::websocket::WsPayload;
use crate::domain::error::Result;
use crate::domain::url_matcher::UrlMatcher;

const MOCK: &str = include_str!("scripts/websocket_mock.js");
pub(crate) const BINDING: &str = "__pwWebSocketBinding";

/// Runs a `__pwWebSocketDispatch(...)` source in the socket's frame.
pub(crate) type Dispatcher = Arc<dyn Fn(String) -> BoxFuture<'static, ()> + Send + Sync>;

pub(crate) fn mock_init_script() -> String {
    format!(
        "(() => {{\nconst module = {{}};\n{MOCK}\n(module.exports.inject())(globalThis);\n}})();"
    )
}

pub(crate) type RouteHandler = Arc<dyn Fn(WebSocketRoute) -> BoxFuture<'static, ()> + Send + Sync>;
type MessageHandler = Arc<dyn Fn(WsPayload) -> BoxFuture<'static, ()> + Send + Sync>;
type CloseHandler = Arc<dyn Fn(Option<u16>, String) -> BoxFuture<'static, ()> + Send + Sync>;

pub(crate) struct WsRouteEntry {
    matcher: UrlMatcher,
    handler: RouteHandler,
}

pub(crate) type WsRoutes = Mutex<Vec<Arc<WsRouteEntry>>>;

impl From<&str> for WsPayload {
    fn from(s: &str) -> Self {
        WsPayload::Text(s.to_owned())
    }
}

impl From<String> for WsPayload {
    fn from(s: String) -> Self {
        WsPayload::Text(s)
    }
}

impl From<Vec<u8>> for WsPayload {
    fn from(b: Vec<u8>) -> Self {
        WsPayload::Binary(b)
    }
}

fn payload_to_data(p: &WsPayload) -> Value {
    match p {
        WsPayload::Text(t) => json!({"data": t, "isBase64": false}),
        WsPayload::Binary(b) => json!({
            "data": base64::engine::general_purpose::STANDARD.encode(b),
            "isBase64": true,
        }),
    }
}

fn data_to_payload(d: &Value) -> WsPayload {
    let data = d.get("data").and_then(Value::as_str).unwrap_or_default();
    if d.get("isBase64").and_then(Value::as_bool) == Some(true) {
        WsPayload::Binary(
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap_or_default(),
        )
    } else {
        WsPayload::Text(data.to_owned())
    }
}

/// A routed WebSocket (the page side). Cheap to clone.
#[derive(Clone)]
pub struct WebSocketRoute {
    inner: Arc<WsRouteInner>,
}

struct WsRouteInner {
    dispatcher: Dispatcher,
    id: String,
    url: String,
    protocols: Vec<String>,
    state: Mutex<WsRouteState>,
}

#[derive(Default)]
struct WsRouteState {
    connected: bool,
    on_page_message: Option<MessageHandler>,
    on_page_close: Option<CloseHandler>,
    on_server_message: Option<MessageHandler>,
    on_server_close: Option<CloseHandler>,
}

impl std::fmt::Debug for WebSocketRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocketRoute")
            .field("url", &self.inner.url)
            .finish()
    }
}

/// The server side of a [`WebSocketRoute`] after `connect_to_server()`.
#[derive(Clone, Debug)]
pub struct WebSocketServer {
    route: WebSocketRoute,
}

fn message_handler<F, Fut>(f: F) -> MessageHandler
where
    F: Fn(WsPayload) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Arc::new(move |m| f(m).boxed())
}

fn close_handler<F, Fut>(f: F) -> CloseHandler
where
    F: Fn(Option<u16>, String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Arc::new(move |c, r| f(c, r).boxed())
}

impl WebSocketRoute {
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn protocols(&self) -> &[String] {
        &self.inner.protocols
    }

    /// A routed socket of the mock with this id (any engine's frame).
    pub(crate) fn new(
        dispatcher: Dispatcher,
        id: String,
        url: String,
        protocols: Vec<String>,
    ) -> Self {
        WebSocketRoute {
            inner: Arc::new(WsRouteInner {
                dispatcher,
                id,
                url,
                protocols,
                state: Mutex::new(WsRouteState::default()),
            }),
        }
    }

    /// Whether `connect_to_server` ran.
    pub(crate) fn is_connected(&self) -> bool {
        self.inner.state.lock().unwrap().connected
    }

    pub(crate) async fn dispatch(&self, mut request: Value) {
        request["id"] = json!(self.inner.id);
        let source = format!("() => globalThis.__pwWebSocketDispatch({request})");
        (self.inner.dispatcher)(source).await;
    }

    /// Sends a message to the page.
    pub async fn send(&self, message: impl Into<WsPayload>) {
        let data = payload_to_data(&message.into());
        self.dispatch(json!({"type": "sendToPage", "data": data}))
            .await;
    }

    /// Closes the page's socket.
    pub async fn close(&self, code: Option<u16>, reason: Option<&str>) {
        self.dispatch(
            json!({"type": "closePage", "code": code, "reason": reason, "wasClean": true}),
        )
        .await;
    }

    /// Handles messages the page sends (instead of forwarding them).
    pub fn on_message<F, Fut>(&self, handler: F)
    where
        F: Fn(WsPayload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.inner.state.lock().unwrap().on_page_message = Some(message_handler(handler));
    }

    /// Handles the page closing its socket (code, reason).
    pub fn on_close<F, Fut>(&self, handler: F)
    where
        F: Fn(Option<u16>, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.inner.state.lock().unwrap().on_page_close = Some(close_handler(handler));
    }

    /// Opens the real connection; unhandled messages flow both ways.
    pub fn connect_to_server(&self) -> Result<WebSocketServer> {
        {
            let mut st = self.inner.state.lock().unwrap();
            if st.connected {
                return Err(crate::domain::error::Error::InvalidArgument(
                    "Already connected to the server".into(),
                ));
            }
            st.connected = true;
        }
        let me = self.clone();
        tokio::spawn(async move { me.dispatch(json!({"type": "connect"})).await });
        Ok(WebSocketServer {
            route: self.clone(),
        })
    }

    pub(crate) async fn on_binding(&self, payload: &Value) {
        let kind = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let code = payload
            .get("code")
            .and_then(Value::as_u64)
            .map(|c| c as u16);
        let reason = payload
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let (page_msg, page_close, server_msg, server_close, connected) = {
            let st = self.inner.state.lock().unwrap();
            (
                st.on_page_message.clone(),
                st.on_page_close.clone(),
                st.on_server_message.clone(),
                st.on_server_close.clone(),
                st.connected,
            )
        };
        let wire = json!({"code": code, "reason": reason, "wasClean": payload.get("wasClean")});
        match kind {
            "onMessageFromPage" => {
                let data = payload.get("data").cloned().unwrap_or(Value::Null);
                match page_msg {
                    Some(h) => h(data_to_payload(&data)).await,
                    None if connected => {
                        self.dispatch(json!({"type": "sendToServer", "data": data}))
                            .await
                    }
                    None => {}
                }
            }
            "onMessageFromServer" => {
                let data = payload.get("data").cloned().unwrap_or(Value::Null);
                match server_msg {
                    Some(h) => h(data_to_payload(&data)).await,
                    None => {
                        self.dispatch(json!({"type": "sendToPage", "data": data}))
                            .await
                    }
                }
            }
            "onClosePage" => match page_close {
                Some(h) => h(code, reason).await,
                None => {
                    let mut req = wire;
                    req["type"] = json!("closeServer");
                    self.dispatch(req).await
                }
            },
            "onCloseServer" => match server_close {
                Some(h) => h(code, reason).await,
                None => {
                    let mut req = wire;
                    req["type"] = json!("closePage");
                    self.dispatch(req).await
                }
            },
            _ => {}
        }
    }
}

impl WebSocketServer {
    pub fn url(&self) -> &str {
        self.route.url()
    }

    /// Sends a message to the server.
    pub async fn send(&self, message: impl Into<WsPayload>) {
        let data = payload_to_data(&message.into());
        self.route
            .dispatch(json!({"type": "sendToServer", "data": data}))
            .await;
    }

    /// Closes the server connection.
    pub async fn close(&self, code: Option<u16>, reason: Option<&str>) {
        self.route
            .dispatch(
                json!({"type": "closeServer", "code": code, "reason": reason, "wasClean": true}),
            )
            .await;
    }

    /// Handles messages from the server (instead of forwarding them).
    pub fn on_message<F, Fut>(&self, handler: F)
    where
        F: Fn(WsPayload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.route.inner.state.lock().unwrap().on_server_message = Some(message_handler(handler));
    }

    /// Handles the server closing the connection.
    pub fn on_close<F, Fut>(&self, handler: F)
    where
        F: Fn(Option<u16>, String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.route.inner.state.lock().unwrap().on_server_close = Some(close_handler(handler));
    }
}

/// Live routed sockets of a context, by mock id.
#[derive(Default)]
pub(crate) struct WsRouting {
    installed: bool,
    sockets: HashMap<String, WebSocketRoute>,
}

/// The `protocols` of an `onCreate` binding payload.
pub(crate) fn protocols_of(payload: &Value) -> Vec<String> {
    payload
        .get("protocols")
        .and_then(Value::as_array)
        .map(|p| {
            p.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn find_handler(routes: &WsRoutes, url: &str) -> Option<RouteHandler> {
    routes
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|e| e.matcher.matches(url))
        .map(|e| e.handler.clone())
}

async fn on_binding(context: Arc<ContextInner>, source: BindingSource, payload: Value) {
    let kind = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if kind == "onCreate" {
        let url = payload
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let handler = find_handler(&source.page.inner.ws_routes, &url)
            .or_else(|| find_handler(&context.ws_routes, &url));
        let Some(handler) = handler else {
            let src = format!(
                "() => globalThis.__pwWebSocketDispatch({})",
                json!({"id": id, "type": "passthrough"})
            );
            let _ = source.frame.evaluate::<Value>(&src).await;
            return;
        };
        let frame = source.frame.clone();
        let route = WebSocketRoute::new(
            Arc::new(move |src: String| {
                let frame = frame.clone();
                async move {
                    let _ = frame.evaluate::<Value>(&src).await;
                }
                .boxed()
            }),
            id.clone(),
            url,
            protocols_of(&payload),
        );
        context
            .ws_routing
            .lock()
            .unwrap()
            .sockets
            .insert(id, route.clone());
        tokio::spawn(async move {
            handler(route.clone()).await;
            if !route.inner.state.lock().unwrap().connected {
                route.dispatch(json!({"type": "ensureOpened"})).await;
            }
        });
        return;
    }
    let route = context.ws_routing.lock().unwrap().sockets.get(&id).cloned();
    if let Some(route) = route {
        tokio::spawn(async move { route.on_binding(&payload).await });
    }
}

impl BrowserContext {
    /// Installs the binding once per context.
    async fn ensure_ws_binding(&self) -> Result<()> {
        {
            let mut r = self.inner.ws_routing.lock().unwrap();
            if r.installed {
                return Ok(());
            }
            r.installed = true;
        }
        let weak = Arc::downgrade(&self.inner);
        self.expose_binding(BINDING, move |source: BindingSource, args: Vec<Value>| {
            let weak = weak.clone();
            async move {
                if let (Some(ctx), Some(payload)) = (weak.upgrade(), args.into_iter().next()) {
                    tokio::spawn(on_binding(ctx, source, payload));
                }
                Ok(Value::Null)
            }
        })
        .await
    }

    /// `context.routeWebSocket(url, handler)`: applies to pages loaded afterwards.
    pub async fn route_web_socket<F, Fut>(
        &self,
        url: impl Into<UrlMatcher>,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(WebSocketRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.ensure_ws_binding().await?;
        self.inner
            .ws_routes
            .lock()
            .unwrap()
            .push(Arc::new(WsRouteEntry {
                matcher: url.into(),
                handler: Arc::new(move |r| handler(r).boxed()),
            }));
        self.add_init_script(&mock_init_script()).await
    }
}

impl Page {
    /// `page.routeWebSocket(url, handler)`: applies to documents loaded afterwards.
    pub async fn route_web_socket<F, Fut>(
        &self,
        url: impl Into<UrlMatcher>,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(WebSocketRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let context = self
            .inner
            .context
            .upgrade()
            .map(|inner| BrowserContext { inner })
            .ok_or_else(|| {
                crate::domain::error::Error::TargetClosed("context has been closed".into())
            })?;
        context.ensure_ws_binding().await?;
        self.inner
            .ws_routes
            .lock()
            .unwrap()
            .push(Arc::new(WsRouteEntry {
                matcher: url.into(),
                handler: Arc::new(move |r| handler(r).boxed()),
            }));
        self.add_init_script(&mock_init_script()).await
    }
}
