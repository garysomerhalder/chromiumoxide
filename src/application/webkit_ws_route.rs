//! `routeWebSocket` on WebKit: Playwright's WebSocket mock (as on
//! Chromium) as an init script, reporting to a context binding; the routed
//! sockets are Chromium's `WebSocketRoute`, answering through the frame.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use serde_json::{Value, json};

use crate::application::webkit::{BrowserContext, ContextInner, Page};
use crate::application::webkit_binding::BindingSource;
use crate::application::websocket_route::{
    BINDING, RouteHandler, WebSocketRoute, mock_init_script, protocols_of,
};
use crate::domain::error::{Error, Result};
use crate::domain::url_matcher::UrlMatcher;

/// `routeWebSocket` registrations of a page or context, newest last.
pub(crate) type WkWsRoutes = Mutex<Vec<(UrlMatcher, RouteHandler)>>;

/// The routed sockets of a context, by mock id.
#[derive(Default)]
pub(crate) struct WkWsRouting {
    pub(crate) sockets: HashMap<String, WebSocketRoute>,
}

fn find(routes: &WkWsRoutes, url: &str) -> Option<RouteHandler> {
    routes
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(m, _)| m.matches(url))
        .map(|(_, h)| h.clone())
}

async fn on_binding(context: Arc<ContextInner>, source: BindingSource, payload: Value) {
    let kind = payload["type"].as_str().unwrap_or_default();
    let id = payload["id"].as_str().unwrap_or_default().to_owned();
    let frame = source.frame.clone();
    let dispatcher: crate::application::websocket_route::Dispatcher =
        Arc::new(move |src: String| {
            let frame = frame.clone();
            async move {
                let _ = frame.evaluate::<Value>(&src).await;
            }
            .boxed()
        });
    if kind == "onCreate" {
        let url = payload["url"].as_str().unwrap_or_default().to_owned();
        let handler =
            find(&source.page.inner.ws_routes, &url).or_else(|| find(&context.ws_routes, &url));
        let Some(handler) = handler else {
            dispatcher(format!(
                "() => globalThis.__pwWebSocketDispatch({})",
                json!({"id": id, "type": "passthrough"})
            ))
            .await;
            return;
        };
        let route = WebSocketRoute::new(dispatcher, id.clone(), url, protocols_of(&payload));
        context
            .ws_routing
            .lock()
            .unwrap()
            .sockets
            .insert(id, route.clone());
        tokio::spawn(async move {
            handler(route.clone()).await;
            if !route.is_connected() {
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
    /// Installs the mock's binding and init script once per context.
    async fn ensure_ws_mock(&self) -> Result<()> {
        if self.inner.ws_installed.swap(true, Ordering::SeqCst) {
            return Ok(());
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
        .await?;
        self.add_init_script(&mock_init_script()).await
    }

    /// `context.routeWebSocket(url, handler)`: applies to documents loaded
    /// afterwards.
    pub async fn route_web_socket<F, Fut>(
        &self,
        url: impl Into<UrlMatcher>,
        handler: F,
    ) -> Result<()>
    where
        F: Fn(WebSocketRoute) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.ensure_ws_mock().await?;
        self.inner
            .ws_routes
            .lock()
            .unwrap()
            .push((url.into(), Arc::new(move |r| handler(r).boxed())));
        Ok(())
    }
}

impl Page {
    /// `page.routeWebSocket(url, handler)`: applies to documents loaded
    /// afterwards.
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
            .context()
            .ok_or_else(|| Error::TargetClosed("context has been closed".into()))?;
        context.ensure_ws_mock().await?;
        self.inner
            .ws_routes
            .lock()
            .unwrap()
            .push((url.into(), Arc::new(move |r| handler(r).boxed())));
        Ok(())
    }
}
