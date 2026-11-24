//! `page.routeFromHAR` / `context.routeFromHAR` on WebKit: answers
//! matching requests from a HAR file through routes (Chromium's lookup and
//! serving rules), or with `update`, records the traffic into it.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::application::webkit::{BrowserContext, Page};
use crate::application::webkit_route::Route;
use crate::domain::error::{Error, Result};
use crate::domain::har::{Har, HarContent, HarMode, HarNotFound, Log, RecordHar};
use crate::domain::url_matcher::UrlMatcher;

/// A pending `routeFromHAR` on WebKit.
#[must_use = "does nothing until awaited"]
pub struct RouteFromHar<'a> {
    page: Option<&'a Page>,
    context: Option<&'a BrowserContext>,
    path: PathBuf,
    url: Option<UrlMatcher>,
    not_found: HarNotFound,
    update: bool,
    update_content: HarContent,
    update_mode: HarMode,
}

impl RouteFromHar<'_> {
    /// Only serve requests matching this URL from the HAR.
    pub fn url(mut self, url: impl Into<UrlMatcher>) -> Self {
        self.url = Some(url.into());
        self
    }
    /// What to do with requests that have no entry.
    pub fn not_found(mut self, mode: HarNotFound) -> Self {
        self.not_found = mode;
        self
    }
    /// Playwright's `update`: record the traffic into the HAR instead of
    /// serving from it (written when the context closes).
    pub fn update(mut self, update: bool) -> Self {
        self.update = update;
        self
    }
    /// Playwright's `updateContent` (default `Attach`).
    pub fn update_content(mut self, content: HarContent) -> Self {
        self.update_content = content;
        self
    }
    /// Playwright's `updateMode` (default `Minimal`).
    pub fn update_mode(mut self, mode: HarMode) -> Self {
        self.update_mode = mode;
        self
    }
}

impl Page {
    /// `page.routeFromHAR(path)`: answers matching requests from a HAR file.
    pub fn route_from_har(&self, path: impl Into<PathBuf>) -> RouteFromHar<'_> {
        RouteFromHar::new(Some(self), None, path.into())
    }
}

impl BrowserContext {
    /// `context.routeFromHAR(path)`.
    pub fn route_from_har(&self, path: impl Into<PathBuf>) -> RouteFromHar<'_> {
        RouteFromHar::new(None, Some(self), path.into())
    }
}

impl<'a> RouteFromHar<'a> {
    fn new(page: Option<&'a Page>, context: Option<&'a BrowserContext>, path: PathBuf) -> Self {
        RouteFromHar {
            page,
            context,
            path,
            url: None,
            not_found: HarNotFound::Abort,
            update: false,
            update_content: HarContent::Attach,
            update_mode: HarMode::Minimal,
        }
    }
}

/// Where a HAR's attached bodies are read from.
struct HarBase {
    fs: Arc<dyn crate::ports::FileSystem>,
    dir: PathBuf,
}

async fn serve(route: Route, log: Arc<Log>, not_found: HarNotFound, base: Arc<HarBase>) {
    let request = route.request();
    let post = request.post_data_buffer().await.ok().flatten();
    let entry = log
        .find(request.method(), request.url(), post.as_deref())
        .filter(|e| e.response.status >= 0)
        .cloned();
    let _ = match entry {
        Some(e) => {
            let mut fulfill = route.fulfill().status(e.response.status as u16);
            for h in &e.response.headers {
                if !h.name.eq_ignore_ascii_case("content-length")
                    && !h.name.eq_ignore_ascii_case("content-encoding")
                {
                    fulfill = fulfill.header(h.name.clone(), h.value.clone());
                }
            }
            fulfill
                .body(match &e.response.content.file {
                    Some(file) => base.fs.read(&base.dir.join(file)).unwrap_or_default(),
                    None => e.response.content.body(),
                })
                .await
        }
        None => match not_found {
            HarNotFound::Abort => route.abort().await,
            HarNotFound::Fallback => route.fallback().await,
        },
    };
}

impl<'a> IntoFuture for RouteFromHar<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let context = match (self.page, self.context) {
                (Some(p), _) => p.context().ok_or_else(|| {
                    Error::TargetClosed("page.routeFromHAR: the page has no context".into())
                })?,
                (_, Some(c)) => c.clone(),
                _ => unreachable!("RouteFromHar always has a target"),
            };
            let fs = context.inner.browser.inner.services.fs.clone();
            if self.update {
                let options = RecordHar {
                    content: Some(self.update_content),
                    mode: self.update_mode,
                    url_filter: None,
                    ..RecordHar::new(&self.path)
                };
                context.inner.recorders.start_har(options).await?;
                for page in context.live_pages() {
                    context.inner.recorders.har_page(&page);
                }
                return Ok(());
            }
            let bytes = fs.read(&self.path)?;
            let har: Har = serde_json::from_slice(&bytes).map_err(|e| {
                Error::InvalidArgument(format!(
                    "Error reading HAR file {}: {e}",
                    self.path.display()
                ))
            })?;
            let log = Arc::new(har.log);
            let base = Arc::new(HarBase {
                fs,
                dir: self.path.parent().map(PathBuf::from).unwrap_or_default(),
            });
            let url = self.url.unwrap_or_else(|| UrlMatcher::glob("**/*"));
            let not_found = self.not_found;
            let handler = move |r: Route| {
                let log = log.clone();
                let base = base.clone();
                async move { serve(r, log, not_found, base).await }
            };
            match (self.page, self.context) {
                (Some(p), _) => p.route(url, handler).await,
                (_, Some(c)) => c.route(url, handler).await,
                _ => Ok(()),
            }
        }
        .boxed()
    }
}
