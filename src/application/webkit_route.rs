//! Request routing on the WebKit engine (`page.route`), as Playwright's
//! `WKRouteImpl`: `Network.addInterception` on each page target; every
//! intercepted request goes to the newest matching handler, which fulfils,
//! continues or aborts it.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;

use crate::application::firefox_route::Request;
use crate::application::webkit::Page;
use crate::domain::error::{Error, Result};
use crate::domain::url_matcher::UrlMatcher;

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

fn todo() -> Error {
    Error::InvalidArgument("not implemented on WebKit yet".into())
}

impl Page {
    /// `page.route(url, handler)`: handlers registered later run first.
    pub async fn route<F, Fut>(&self, _url: impl Into<UrlMatcher>, _handler: F) -> Result<()>
    where
        F: Fn(Route) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        Err(todo())
    }

    /// `page.unroute(url)`: removes the handlers registered for `url`.
    pub async fn unroute(&self, _url: impl Into<UrlMatcher>) -> Result<()> {
        Err(todo())
    }
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

    /// `route.continue()`.
    pub async fn continue_(&self) -> Result<()> {
        self.take()?;
        Err(todo())
    }

    /// `route.abort()`.
    pub async fn abort(&self) -> Result<()> {
        self.take()?;
        Err(todo())
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
            self.route.take()?;
            let _ = (self.status, self.headers, self.content_type, self.body);
            Err(todo())
        })
    }
}
