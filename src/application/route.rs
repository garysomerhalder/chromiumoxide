//! Request routing: `page.route()` / `context.route()` with fulfill, abort,
//! continue and fallback, over the protocol's `Fetch` domain.

use std::future::{Future, IntoFuture};
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::network::Request;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::network::{
    HeaderEntry, error_reason, fulfill_headers, split_set_cookie, status_text,
};
use crate::domain::url_matcher::UrlMatcher;

type HandlerFn = Arc<dyn Fn(Route) -> BoxFuture<'static, ()> + Send + Sync>;

/// A registered route handler.
pub(crate) struct RouteEntry {
    matcher: UrlMatcher,
    key: String,
    handler: HandlerFn,
    times_left: Mutex<Option<usize>>,
}

impl RouteEntry {
    /// Claims one use; false when the handler has expired.
    fn claim(&self) -> bool {
        let mut left = self.times_left.lock().unwrap();
        match left.as_mut() {
            None => true,
            Some(0) => false,
            Some(n) => {
                *n -= 1;
                true
            }
        }
    }

    fn expired(&self) -> bool {
        *self.times_left.lock().unwrap() == Some(0)
    }
}

pub(crate) type Routes = Mutex<Vec<Arc<RouteEntry>>>;

fn matcher_key(m: &UrlMatcher) -> String {
    match m {
        UrlMatcher::Glob { glob, .. } => format!("glob:{glob}"),
        UrlMatcher::Regex(r) => format!("regex:{}", r.as_str()),
    }
}

#[derive(Default, Clone)]
struct Overrides {
    url: Option<String>,
    method: Option<String>,
    headers: Option<Vec<HeaderEntry>>,
    post_data: Option<Vec<u8>>,
}

impl Overrides {
    fn merge(&mut self, other: Overrides) {
        if other.url.is_some() {
            self.url = other.url;
        }
        if other.method.is_some() {
            self.method = other.method;
        }
        if other.headers.is_some() {
            self.headers = other.headers;
        }
        if other.post_data.is_some() {
            self.post_data = other.post_data;
        }
    }
}

enum Decision {
    Handled,
    Fallback,
}

/// An intercepted request waiting for a decision (Playwright's `Route`).
#[derive(Clone)]
pub struct Route {
    inner: Arc<RouteInner>,
}

struct RouteInner {
    page: Page,
    interception_id: String,
    request: Request,
    handled: Mutex<bool>,
    overrides: Mutex<Overrides>,
    decision: Mutex<Option<oneshot::Sender<Decision>>>,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("request", &self.inner.request)
            .finish()
    }
}

impl Route {
    /// The intercepted request.
    pub fn request(&self) -> Request {
        self.inner.request.clone()
    }

    /// `route.fulfill()`: answers with a made-up response.
    pub fn fulfill(&self) -> Fulfill<'_> {
        Fulfill {
            route: self,
            status: None,
            headers: Vec::new(),
            content_type: None,
            body: None,
            json: None,
        }
    }

    /// `route.abort()`: fails the request (`failed` unless an error code is given).
    pub fn abort(&self) -> Abort<'_> {
        Abort {
            route: self,
            code: "failed".into(),
        }
    }

    /// `route.continue()`: sends the request on, optionally changed.
    pub fn continue_(&self) -> Continue<'_> {
        Continue {
            route: self,
            overrides: Overrides::default(),
            fallback: false,
        }
    }

    /// `route.fallback()`: hands the request to the next matching handler.
    pub fn fallback(&self) -> Continue<'_> {
        Continue {
            route: self,
            overrides: Overrides::default(),
            fallback: true,
        }
    }

    fn take_handled(&self) -> Result<()> {
        let mut handled = self.inner.handled.lock().unwrap();
        if *handled {
            return Err(Error::InvalidArgument("Route is already handled!".into()));
        }
        *handled = true;
        Ok(())
    }

    fn decide(&self, d: Decision) {
        if let Some(tx) = self.inner.decision.lock().unwrap().take() {
            let _ = tx.send(d);
        }
    }

    async fn send(&self, method: &str, mut params: Value) -> Result<()> {
        params["requestId"] = json!(self.inner.interception_id);
        match self.inner.page.send(method, params).await {
            Ok(_) => Ok(()),
            // The request went away (page closed or navigated): nothing to do.
            Err(Error::Protocol { message, .. })
                if message.contains("Invalid InterceptionId")
                    || message.contains("Invalid state") =>
            {
                Ok(())
            }
            Err(Error::TargetClosed(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn resume(&self) -> Result<()> {
        let o = self.inner.overrides.lock().unwrap().clone();
        let mut params = json!({});
        if let Some(url) = o.url {
            params["url"] = json!(url);
        }
        if let Some(method) = o.method {
            params["method"] = json!(method);
        }
        if let Some(headers) = o.headers {
            let list: Vec<Value> = headers
                .iter()
                .filter(|h| !h.name.eq_ignore_ascii_case("cookie"))
                .map(|h| json!({"name": h.name, "value": h.value}))
                .collect();
            params["headers"] = json!(list);
        }
        if let Some(body) = o.post_data {
            params["postData"] = json!(base64::engine::general_purpose::STANDARD.encode(body));
        }
        self.send("Fetch.continueRequest", params).await
    }
}

/// A pending `route.fulfill()`.
#[must_use = "does nothing until awaited"]
pub struct Fulfill<'a> {
    route: &'a Route,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    content_type: Option<String>,
    body: Option<Vec<u8>>,
    json: Option<serde_json::Result<String>>,
}

impl<'a> Fulfill<'a> {
    pub fn status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn headers<K: Into<String>, V: Into<String>>(
        mut self,
        headers: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.headers = headers
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(body.into());
        self
    }
    /// A JSON body (and `content-type: application/json`).
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        self.json = Some(serde_json::to_string(value));
        self
    }
}

impl<'a> IntoFuture for Fulfill<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let is_json = self.json.is_some();
            let body = match self.json {
                Some(json) => {
                    if self.body.is_some() {
                        return Err(Error::InvalidArgument(
                            "Can specify either body or json parameters".into(),
                        ));
                    }
                    json?.into_bytes()
                }
                None => self.body.unwrap_or_default(),
            };
            let status = self.status.unwrap_or(200);
            let headers = split_set_cookie(fulfill_headers(
                &self.headers,
                self.content_type.as_deref(),
                is_json,
                body.len(),
            ));
            self.route.take_handled()?;
            self.route.decide(Decision::Handled);
            let list: Vec<Value> = headers
                .iter()
                .map(|h| json!({"name": h.name, "value": h.value}))
                .collect();
            self.route
                .send(
                    "Fetch.fulfillRequest",
                    json!({
                        "responseCode": status,
                        "responsePhrase": status_text(status),
                        "responseHeaders": list,
                        "body": base64::engine::general_purpose::STANDARD.encode(&body),
                    }),
                )
                .await
        }
        .boxed()
    }
}

/// A pending `route.abort()`.
#[must_use = "does nothing until awaited"]
pub struct Abort<'a> {
    route: &'a Route,
    code: String,
}

impl<'a> Abort<'a> {
    /// `aborted`, `accessdenied`, `connectionrefused`, `timedout`, `failed`, ...
    pub fn error_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }
}

impl<'a> IntoFuture for Abort<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let reason = error_reason(&self.code).ok_or_else(|| {
                Error::InvalidArgument(format!("Unknown error code: {}", self.code))
            })?;
            self.route.take_handled()?;
            self.route.decide(Decision::Handled);
            self.route
                .send("Fetch.failRequest", json!({"errorReason": reason}))
                .await
        }
        .boxed()
    }
}

/// A pending `route.continue()` or `route.fallback()`, with optional overrides.
#[must_use = "does nothing until awaited"]
pub struct Continue<'a> {
    route: &'a Route,
    overrides: Overrides,
    fallback: bool,
}

impl<'a> Continue<'a> {
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.overrides.url = Some(url.into());
        self
    }
    pub fn method(mut self, method: impl Into<String>) -> Self {
        self.overrides.method = Some(method.into());
        self
    }
    /// Replaces all request headers.
    pub fn headers<K: Into<String>, V: Into<String>>(
        mut self,
        headers: impl IntoIterator<Item = (K, V)>,
    ) -> Self {
        self.overrides.headers = Some(
            headers
                .into_iter()
                .map(|(k, v)| HeaderEntry::new(k, v))
                .collect(),
        );
        self
    }
    pub fn post_data(mut self, data: impl Into<Vec<u8>>) -> Self {
        self.overrides.post_data = Some(data.into());
        self
    }
}

impl<'a> IntoFuture for Continue<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            self.route.take_handled()?;
            self.route
                .inner
                .overrides
                .lock()
                .unwrap()
                .merge(self.overrides);
            if self.fallback {
                // The next handler gets a fresh chance to handle the route.
                *self.route.inner.handled.lock().unwrap() = false;
                self.route.decide(Decision::Fallback);
                Ok(())
            } else {
                self.route.decide(Decision::Handled);
                self.route.resume().await
            }
        }
        .boxed()
    }
}

enum Target<'a> {
    Page(&'a Page),
    Context(&'a BrowserContext),
}

/// A pending `route()` registration.
#[must_use = "does nothing until awaited"]
pub struct RouteRegistration<'a> {
    target: Target<'a>,
    matcher: UrlMatcher,
    handler: HandlerFn,
    times: Option<usize>,
}

impl<'a> RouteRegistration<'a> {
    /// Handle only the first `n` matching requests.
    pub fn times(mut self, n: usize) -> Self {
        self.times = Some(n);
        self
    }
}

impl<'a> IntoFuture for RouteRegistration<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let entry = Arc::new(RouteEntry {
                key: matcher_key(&self.matcher),
                matcher: self.matcher,
                handler: self.handler,
                times_left: Mutex::new(self.times),
            });
            match self.target {
                Target::Page(page) => {
                    page.inner.routes.lock().unwrap().push(entry);
                    page.update_interception().await
                }
                Target::Context(ctx) => {
                    ctx.inner.routes.lock().unwrap().push(entry);
                    ctx.update_interception().await
                }
            }
        }
        .boxed()
    }
}

fn boxed_handler<F, Fut>(handler: F) -> HandlerFn
where
    F: Fn(Route) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    Arc::new(move |r| handler(r).boxed())
}

/// Commands that turn on interception for a new page of `context`.
pub(crate) fn context_route_commands(context: &ContextInner) -> Vec<(&'static str, Value)> {
    if context.routes.lock().unwrap().is_empty() || context.options.http_credentials.is_some() {
        return Vec::new();
    }
    vec![
        ("Network.setCacheDisabled", json!({"cacheDisabled": true})),
        (
            "Fetch.enable",
            json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
        ),
    ]
}

impl Page {
    /// `page.route(url, handler)`. Handlers registered later run first.
    pub fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> RouteRegistration<'_>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        RouteRegistration {
            target: Target::Page(self),
            matcher: url.into(),
            handler: boxed_handler(handler),
            times: None,
        }
    }

    /// `page.unroute(url)`: removes the handlers registered for `url`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.inner.routes.lock().unwrap().retain(|e| e.key != key);
        self.update_interception().await
    }

    /// `page.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.inner.routes.lock().unwrap().clear();
        self.update_interception().await
    }

    fn context_inner(&self) -> Option<Arc<ContextInner>> {
        self.inner.context.upgrade()
    }

    fn routing_active(&self) -> bool {
        !self.inner.routes.lock().unwrap().is_empty()
            || self
                .context_inner()
                .is_some_and(|c| !c.routes.lock().unwrap().is_empty())
    }

    /// Turns `Fetch` interception on or off to match the registered routes.
    pub(crate) async fn update_interception(&self) -> Result<()> {
        let want = self.routing_active() || self.inner.options.http_credentials.is_some();
        let was = self
            .inner
            .intercepting
            .swap(want, std::sync::atomic::Ordering::SeqCst);
        if want && !was {
            self.send("Network.setCacheDisabled", json!({"cacheDisabled": true}))
                .await?;
            self.send(
                "Fetch.enable",
                json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
            )
            .await?;
        } else if !want && was {
            self.send("Fetch.disable", json!({})).await?;
            self.send("Network.setCacheDisabled", json!({"cacheDisabled": false}))
                .await?;
        }
        Ok(())
    }

    /// `Fetch.requestPaused`: pairs it with its request, or continues it.
    pub(crate) fn on_request_paused(&self, p: &Value) {
        let interception_id = p
            .get("requestId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let network_id = p.get("networkId").and_then(Value::as_str);
        let url = p
            .pointer("/request/url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(network_id) = network_id.filter(|_| self.routing_active()) else {
            let fut = self.inner.shared.cdp.call(
                Some(&self.inner.session_id),
                "Fetch.continueRequest",
                json!({"requestId": interception_id}),
            );
            tokio::spawn(fut);
            return;
        };
        if url.starts_with("data:") {
            return;
        }
        let request = {
            let mut net = self.inner.network.lock().unwrap();
            match net.request(network_id) {
                Some(r) if r.url() == url || r.url().split('#').next() == Some(url) => Some(r),
                _ => {
                    net.stash_paused(network_id, interception_id.clone());
                    None
                }
            }
        };
        if let Some(request) = request {
            self.dispatch_route(request, interception_id);
        }
    }

    /// Runs the matching handlers (page first, newest first), then continues.
    pub(crate) fn dispatch_route(&self, request: Request, interception_id: String) {
        let mut chain: Vec<Arc<RouteEntry>> = Vec::new();
        {
            let page_routes = self.inner.routes.lock().unwrap();
            chain.extend(page_routes.iter().rev().cloned());
        }
        if let Some(ctx) = self.context_inner() {
            chain.extend(ctx.routes.lock().unwrap().iter().rev().cloned());
        }
        let route = Route {
            inner: Arc::new(RouteInner {
                page: self.clone(),
                interception_id,
                request,
                handled: Mutex::new(false),
                overrides: Mutex::new(Overrides::default()),
                decision: Mutex::new(None),
            }),
        };
        let page = self.clone();
        tokio::spawn(async move {
            for entry in chain {
                if !entry.matcher.matches(route.inner.request.url()) || !entry.claim() {
                    continue;
                }
                if entry.expired() {
                    page.forget_route(&entry);
                }
                let (tx, rx) = oneshot::channel();
                *route.inner.decision.lock().unwrap() = Some(tx);
                tokio::spawn((entry.handler)(route.clone()));
                match rx.await {
                    Ok(Decision::Handled) => return,
                    Ok(Decision::Fallback) | Err(_) => continue,
                }
            }
            if !*route.inner.handled.lock().unwrap() {
                *route.inner.handled.lock().unwrap() = true;
                let _ = route.resume().await;
            }
        });
    }

    fn forget_route(&self, entry: &Arc<RouteEntry>) {
        self.inner
            .routes
            .lock()
            .unwrap()
            .retain(|e| !Arc::ptr_eq(e, entry));
        if let Some(ctx) = self.context_inner() {
            ctx.routes
                .lock()
                .unwrap()
                .retain(|e| !Arc::ptr_eq(e, entry));
        }
    }
}

impl BrowserContext {
    /// `context.route(url, handler)`: applies to every page of the context.
    pub fn route<F, Fut>(&self, url: impl Into<UrlMatcher>, handler: F) -> RouteRegistration<'_>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        RouteRegistration {
            target: Target::Context(self),
            matcher: url.into(),
            handler: boxed_handler(handler),
            times: None,
        }
    }

    /// `context.unroute(url)`.
    pub async fn unroute(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let key = matcher_key(&url.into());
        self.inner.routes.lock().unwrap().retain(|e| e.key != key);
        self.update_interception().await
    }

    /// `context.unrouteAll()`.
    pub async fn unroute_all(&self) -> Result<()> {
        self.inner.routes.lock().unwrap().clear();
        self.update_interception().await
    }

    async fn update_interception(&self) -> Result<()> {
        let pages: Vec<Page> = self.inner.pages.lock().unwrap().clone();
        for page in pages {
            if !page.is_closed() {
                page.update_interception().await?;
            }
        }
        Ok(())
    }
}
