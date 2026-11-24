//! Request routing on the WebKit engine (`page.route`), as Playwright's
//! `WKRouteImpl`: `Network.addInterception` on each page target; every
//! intercepted request goes to the newest matching handler, which fulfils,
//! continues or aborts it.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::firefox_route::{Request, matcher_key};
use crate::application::webkit::Page;
use crate::domain::error::{Error, Result};
use crate::domain::network::status_text;
use crate::domain::url_matcher::UrlMatcher;

/// A route handler, boxed.
pub(crate) type Handler = Arc<dyn Fn(Route) -> BoxFuture<'static, ()> + Send + Sync>;

/// One `page.route` registration.
pub(crate) struct RouteEntry {
    pub(crate) key: String,
    pub(crate) matcher: UrlMatcher,
    pub(crate) handler: Handler,
}

/// An intercepted request (`page.route` handler argument).
#[derive(Clone)]
pub struct Route {
    pub(crate) page: Page,
    pub(crate) target: String,
    pub(crate) id: String,
    pub(crate) request: Request,
    pub(crate) handled: Arc<AtomicBool>,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Route")
            .field("url", &self.request.url)
            .finish()
    }
}

impl Page {
    /// `page.route(url, handler)`: handlers registered later run first.
    pub async fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.check_open()?;
        let matcher = url.into();
        let handler: Handler = Arc::new(move |route| handler(route).boxed());
        let first = {
            let mut routes = self.inner.routes.lock().unwrap();
            routes.push(RouteEntry {
                key: matcher_key(&matcher),
                matcher,
                handler,
            });
            routes.len() == 1
        };
        if first {
            let target = self.target();
            interception(self, &target, true).await?;
        }
        Ok(())
    }

    /// `page.unroute(url)`: removes the handlers registered for `url`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        let empty = {
            let mut routes = self.inner.routes.lock().unwrap();
            routes.retain(|e| e.key != key);
            routes.is_empty()
        };
        if empty {
            let target = self.target();
            interception(self, &target, false).await?;
        }
        Ok(())
    }

    /// `page.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.inner.routes.lock().unwrap().clear();
        let target = self.target();
        interception(self, &target, false).await
    }

    pub(crate) fn has_routes(&self) -> bool {
        !self.inner.routes.lock().unwrap().is_empty()
    }

    /// `Network.requestIntercepted`: to the newest matching handler, else
    /// on to the network.
    pub(crate) fn on_request_intercepted(&self, target: &str, p: &Value) {
        let id = p["requestId"].as_str().unwrap_or_default().to_owned();
        let r = &p["request"];
        let url = r["url"].as_str().unwrap_or_default().to_owned();
        let handler = self
            .inner
            .routes
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|e| e.matcher.matches(&url))
            .map(|e| e.handler.clone());
        let route = Route {
            page: self.clone(),
            target: target.to_owned(),
            id,
            request: Request {
                url,
                method: r["method"].as_str().unwrap_or("GET").to_owned(),
                headers: r["headers"]
                    .as_object()
                    .map(|h| {
                        h.iter()
                            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                            .collect()
                    })
                    .unwrap_or_default(),
                failure: None,
            },
            handled: Arc::new(AtomicBool::new(false)),
        };
        tokio::spawn(async move {
            match handler {
                Some(h) => {
                    h(route.clone()).await;
                    // A handler that returns without handling the request
                    // lets it go on, as in Playwright's fallback.
                    if !route.handled.load(Ordering::SeqCst) {
                        let _ = route.continue_().await;
                    }
                }
                None => {
                    let _ = route.continue_().await;
                }
            }
        });
    }
}

/// Turns request interception on or off for a page target.
pub(crate) async fn interception(page: &Page, target: &str, enabled: bool) -> Result<()> {
    let wk = &page.inner.wk;
    let proxy = &page.inner.proxy;
    wk.call_target(
        proxy,
        target,
        "Network.setInterceptionEnabled",
        json!({"enabled": enabled}),
    )
    .await?;
    let method = if enabled {
        "Network.addInterception"
    } else {
        "Network.removeInterception"
    };
    wk.call_target(
        proxy,
        target,
        method,
        json!({"url": ".*", "stage": "request", "isRegex": true}),
    )
    .await
    .map(drop)
}

impl Route {
    pub fn request(&self) -> Request {
        self.request.clone()
    }

    fn take(&self) -> Result<()> {
        if self.handled.swap(true, Ordering::SeqCst) {
            return Err(Error::InvalidArgument("Route is already handled!".into()));
        }
        Ok(())
    }

    async fn call(&self, method: &str, params: Value) -> Result<()> {
        let inner = &self.page.inner;
        inner
            .wk
            .call_target(&inner.proxy, &self.target, method, params)
            .await
            .map(drop)
    }

    /// `route.continue()`: sends the request on unchanged.
    pub async fn continue_(&self) -> Result<()> {
        self.take()?;
        self.call(
            "Network.interceptWithRequest",
            json!({"requestId": self.id}),
        )
        .await
    }

    /// `route.abort()`: fails the request (Playwright's `failed`).
    pub async fn abort(&self) -> Result<()> {
        self.take()?;
        self.call(
            "Network.interceptRequestWithError",
            json!({"requestId": self.id, "errorType": "General"}),
        )
        .await
    }

    /// `route.fulfill()`: 200 with an empty body unless set.
    pub fn fulfill(&self) -> Fulfill<'_> {
        Fulfill {
            route: self,
            status: 200,
            headers: Vec::new(),
            content_type: None,
            body: Vec::new(),
        }
    }
}

/// A pending `route.fulfill()`.
#[must_use = "does nothing until awaited"]
pub struct Fulfill<'a> {
    route: &'a Route,
    status: u16,
    headers: Vec<(String, String)>,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Fulfill<'_> {
    pub fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

impl<'a> IntoFuture for Fulfill<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(async move {
            if (300..400).contains(&self.status) {
                return Err(Error::InvalidArgument(format!(
                    "Cannot fulfill with redirect status: {}",
                    self.status
                )));
            }
            self.route.take()?;
            let mut headers = serde_json::Map::new();
            for (n, v) in self.headers {
                headers.insert(n.to_lowercase(), json!(v));
            }
            if let Some(ct) = self.content_type {
                headers.insert("content-type".into(), json!(ct));
            }
            headers
                .entry("content-length")
                .or_insert_with(|| json!(self.body.len().to_string()));
            let mime_type = headers
                .get("content-type")
                .and_then(Value::as_str)
                .map(|ct| ct.split(';').next().unwrap_or_default().trim().to_owned())
                .unwrap_or_else(|| "text/plain".into());
            self.route
                .call(
                    "Network.interceptRequestWithResponse",
                    json!({
                        "requestId": self.route.id,
                        "status": self.status,
                        "statusText": status_text(self.status),
                        "mimeType": mime_type,
                        "headers": headers,
                        "base64Encoded": true,
                        "content": base64::engine::general_purpose::STANDARD.encode(&self.body),
                    }),
                )
                .await
        })
    }
}
