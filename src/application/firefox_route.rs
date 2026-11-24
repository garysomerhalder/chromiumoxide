//! Request routing on the Firefox engine (`page.route`): a BiDi network
//! intercept per routed page; blocked requests go to the newest matching
//! handler, which fulfils, continues or aborts them.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::firefox::Page;
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

/// How `unroute(url)` finds a registration: the glob or regex source.
pub(crate) fn matcher_key(m: &UrlMatcher) -> String {
    match m {
        UrlMatcher::Glob { glob, .. } => format!("glob:{glob}"),
        UrlMatcher::Regex(r) => format!("regex:{}", r.as_str()),
    }
}

impl Page {
    /// `page.route(url, handler)`: handlers registered later run first.
    pub async fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let matcher = url.into();
        let handler: Handler = Arc::new(move |route| handler(route).boxed());
        self.add_route(RouteEntry {
            key: matcher_key(&matcher),
            matcher,
            handler,
        })
        .await
    }

    /// `page.unroute(url)`: removes the handlers registered for `url`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.remove_routes(|e| e.key == key).await
    }

    /// `page.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.remove_routes(|_| true).await
    }
}

impl crate::application::firefox::BrowserContext {
    /// `context.route(url, handler)`: for every page of this context, also
    /// pages opened later; page routes run before the context's.
    pub async fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let matcher = url.into();
        let handler: Handler = Arc::new(move |route| handler(route).boxed());
        self.add_route(RouteEntry {
            key: matcher_key(&matcher),
            matcher,
            handler,
        })
        .await
    }

    /// `context.unroute(url)`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.remove_routes(|e| e.key == key).await
    }

    /// `context.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.remove_routes(|_| true).await
    }
}

pub use crate::application::firefox_network::{Request, Response};

/// The request a route intercepted (same type as [`Request`]).
pub type RouteRequest = Request;

/// An intercepted request (`page.route` handler argument).
#[derive(Clone)]
pub struct Route {
    pub(crate) page: Page,
    pub(crate) id: String,
    pub(crate) request: RouteRequest,
    pub(crate) handled: Arc<AtomicBool>,
    /// The handlers `fallback()` passes the request to, in order.
    pub(crate) rest: Vec<Handler>,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Route")
            .field("url", &self.request.url())
            .finish()
    }
}

impl Route {
    /// Runs the first handler of `chain` (the rest follow on `fallback`);
    /// with none, the request goes on to the network.
    pub(crate) fn dispatch(
        page: Page,
        id: String,
        request: RouteRequest,
        mut chain: Vec<Handler>,
    ) -> BoxFuture<'static, ()> {
        async move {
            if chain.is_empty() {
                let _ = page
                    .bidi_call("network.continueRequest", json!({"request": id}))
                    .await;
                return;
            }
            let handler = chain.remove(0);
            handler(Route {
                page,
                id,
                request,
                handled: Arc::default(),
                rest: chain,
            })
            .await
        }
        .boxed()
    }

    /// `route.fallback()`: lets the next matching handler (an older page
    /// route, then the context's) handle the request, or the network.
    pub async fn fallback(&self) -> Result<()> {
        self.take()?;
        Route::dispatch(
            self.page.clone(),
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

    pub fn request(&self) -> RouteRequest {
        self.request.clone()
    }

    fn take(&self) -> Result<()> {
        if self.handled.swap(true, Ordering::SeqCst) {
            return Err(Error::InvalidArgument("Route is already handled!".into()));
        }
        Ok(())
    }

    /// `route.continue()`: sends the request on unchanged.
    pub async fn continue_(&self) -> Result<()> {
        self.take()?;
        self.page
            .bidi_call("network.continueRequest", json!({"request": self.id}))
            .await
            .map(drop)
    }

    /// `route.abort()`: fails the request.
    pub async fn abort(&self) -> Result<()> {
        self.take()?;
        self.page
            .bidi_call("network.failRequest", json!({"request": self.id}))
            .await
            .map(drop)
    }

    /// `route.fulfill()`: answers with a made-up response (200, empty body
    /// by default).
    pub fn fulfill(&self) -> Fulfill<'_> {
        Fulfill {
            route: self,
            status: 200,
            headers: Vec::new(),
            content_type: None,
            body: Vec::new(),
            response: None,
            body_set: false,
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
    /// `fulfill({ response })`: its status, headers and (unless `body` is
    /// set) body.
    response: Option<crate::application::request::ApiResponse>,
    body_set: bool,
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
        async move {
            self.route.take()?;
            let mut headers = self.headers;
            let mut body = self.body;
            if let Some(response) = &self.response {
                let mut from: Vec<(String, String)> = response
                    .headers_array()
                    .into_iter()
                    .map(|h| (h.name, h.value))
                    .filter(|(k, _)| {
                        !k.eq_ignore_ascii_case("content-length")
                            && !k.eq_ignore_ascii_case("content-encoding")
                            && !k.eq_ignore_ascii_case("transfer-encoding")
                    })
                    .collect();
                from.retain(|(k, _)| !headers.iter().any(|(h, _)| h.eq_ignore_ascii_case(k)));
                headers.extend(from);
                if !self.body_set {
                    body = response.body().await?;
                }
            }
            if let Some(ct) = self.content_type {
                headers.retain(|(n, _)| !n.eq_ignore_ascii_case("content-type"));
                headers.push(("content-type".into(), ct));
            }
            if !headers
                .iter()
                .any(|(n, _)| n.eq_ignore_ascii_case("content-length"))
            {
                headers.push(("content-length".into(), body.len().to_string()));
            }
            let headers: Vec<Value> = headers
                .iter()
                .map(|(n, v)| json!({"name": n, "value": {"type": "string", "value": v}}))
                .collect();
            self.route
                .page
                .bidi_call(
                    "network.provideResponse",
                    json!({
                        "request": self.route.id,
                        "statusCode": self.status,
                        "reasonPhrase": status_text(self.status),
                        "headers": headers,
                        "body": {"type": "base64",
                                 "value": base64::engine::general_purpose::STANDARD.encode(&body)},
                    }),
                )
                .await
                .map(drop)
        }
        .boxed()
    }
}
