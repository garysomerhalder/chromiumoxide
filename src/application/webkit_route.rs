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

use crate::application::firefox_route::matcher_key;
use crate::application::webkit::Page;
use crate::application::webkit_network::Request;
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
    /// The older matching handlers, for `fallback()`.
    pub(crate) rest: Vec<Handler>,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Route")
            .field("url", &self.request.url())
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
        self.inner.routes.lock().unwrap().push(RouteEntry {
            key: matcher_key(&matcher),
            matcher,
            handler,
        });
        let target = self.target();
        interception(self, &target, true).await
    }

    /// `page.unroute(url)`: removes the handlers registered for `url`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.inner.routes.lock().unwrap().retain(|e| e.key != key);
        let target = self.target();
        interception(self, &target, self.has_routes()).await
    }

    /// `page.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.inner.routes.lock().unwrap().clear();
        let target = self.target();
        interception(self, &target, self.has_routes()).await
    }

    /// Whether the page or its context has routes.
    pub(crate) fn has_routes(&self) -> bool {
        !self.inner.routes.lock().unwrap().is_empty()
            || self
                .inner
                .context
                .upgrade()
                .is_some_and(|c| !c.routes.lock().unwrap().is_empty())
    }

    /// `Network.requestIntercepted`: to the newest matching handler, else
    /// on to the network.
    pub(crate) fn on_request_intercepted(&self, target: &str, p: &Value) {
        let id = p["requestId"].as_str().unwrap_or_default().to_owned();
        let request = Request::intercepted(target, p, Arc::downgrade(&self.inner));
        let url = request.url().to_owned();
        // Page routes newest first, then the context's newest first.
        let matching = |routes: &[RouteEntry]| -> Vec<Handler> {
            routes
                .iter()
                .rev()
                .filter(|e| e.matcher.matches(&url))
                .map(|e| e.handler.clone())
                .collect()
        };
        let mut chain = matching(&self.inner.routes.lock().unwrap());
        if let Some(c) = self.inner.context.upgrade() {
            chain.extend(matching(&c.routes.lock().unwrap()));
        }
        tokio::spawn(Route::dispatch(
            self.clone(),
            target.to_owned(),
            id,
            request,
            chain,
        ));
    }
}

/// Turns request interception on or off for a page target.
pub(crate) async fn interception(page: &Page, target: &str, enabled: bool) -> Result<()> {
    {
        let mut on = page.inner.intercepting.lock().unwrap();
        if on.contains(target) == enabled {
            return Ok(());
        }
        if enabled {
            on.insert(target.to_owned());
        } else {
            on.remove(target);
        }
    }
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

impl crate::application::webkit::BrowserContext {
    /// `context.route(url, handler)`: for every page of this context, also
    /// pages opened later; page routes run before the context's.
    pub async fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let matcher = url.into();
        let handler: Handler = Arc::new(move |route| handler(route).boxed());
        self.inner.routes.lock().unwrap().push(RouteEntry {
            key: matcher_key(&matcher),
            matcher,
            handler,
        });
        self.refresh_interception().await
    }

    /// `context.unroute(url)`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.inner.routes.lock().unwrap().retain(|e| e.key != key);
        self.refresh_interception().await
    }

    /// `context.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.inner.routes.lock().unwrap().clear();
        self.refresh_interception().await
    }

    /// Turns interception on or off on each page as its routes say.
    async fn refresh_interception(&self) -> Result<()> {
        for page in self.live_pages() {
            let target = page.target();
            interception(&page, &target, page.has_routes()).await?;
        }
        Ok(())
    }
}

impl Route {
    /// Runs the first handler of `chain` (the rest follow on `fallback`);
    /// with none, the request goes on to the network. A handler that
    /// returns without handling the request lets it go on.
    pub(crate) fn dispatch(
        page: Page,
        target: String,
        id: String,
        request: Request,
        mut chain: Vec<Handler>,
    ) -> BoxFuture<'static, ()> {
        async move {
            let first = (!chain.is_empty()).then(|| chain.remove(0));
            let route = Route {
                page,
                target,
                id,
                request,
                handled: Arc::default(),
                rest: chain,
            };
            match first {
                Some(handler) => {
                    handler(route.clone()).await;
                    if !route.handled.load(Ordering::SeqCst) {
                        let _ = route.continue_().await;
                    }
                }
                None => {
                    let _ = route.continue_().await;
                }
            }
        }
        .boxed()
    }

    /// `route.fallback()`: lets the next matching handler (an older page
    /// route, then the context's) handle the request, or the network.
    pub async fn fallback(&self) -> Result<()> {
        self.take()?;
        Route::dispatch(
            self.page.clone(),
            self.target.clone(),
            self.id.clone(),
            self.request.clone(),
            self.rest.clone(),
        )
        .await;
        Ok(())
    }

    /// `route.fetch()`: performs the request (through the context's API
    /// request context) and returns the response, for `fulfill().response()`.
    pub async fn fetch(&self) -> Result<crate::application::request::ApiResponse> {
        let request = self.page.request();
        let body = self.request.post_data_buffer().await?;
        let mut call = request
            .fetch(self.request.url())
            .method(self.request.method());
        for (k, v) in self.request.headers() {
            if !k.eq_ignore_ascii_case("cookie") && !k.eq_ignore_ascii_case("host") {
                call = call.header(k, v);
            }
        }
        if let Some(body) = body {
            call = call.data_bytes(body);
        }
        call.await
    }

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
            body_set: false,
            response: None,
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
    body_set: bool,
    response: Option<crate::application::request::ApiResponse>,
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
        self.body_set = true;
        self
    }
    /// `fulfill({ response })`: answers with a fetched response's status,
    /// headers and body (each can still be overridden).
    pub fn response(mut self, response: &crate::application::request::ApiResponse) -> Self {
        self.status = response.status();
        self.response = Some(response.clone());
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
            let mut body = self.body;
            let mut headers = serde_json::Map::new();
            if let Some(response) = &self.response {
                for h in response.headers_array() {
                    let k = h.name.to_lowercase();
                    if !matches!(
                        k.as_str(),
                        "content-length" | "content-encoding" | "transfer-encoding"
                    ) {
                        headers.insert(k, json!(h.value));
                    }
                }
                if !self.body_set {
                    body = response.body().await?;
                }
            }
            for (n, v) in self.headers {
                headers.insert(n.to_lowercase(), json!(v));
            }
            if let Some(ct) = self.content_type {
                headers.insert("content-type".into(), json!(ct));
            }
            headers
                .entry("content-length")
                .or_insert_with(|| json!(body.len().to_string()));
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
                        "content": base64::engine::general_purpose::STANDARD.encode(&body),
                    }),
                )
                .await
        })
    }
}
