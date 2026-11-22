//! HAR recording (`recordHar` context option) and replay (`routeFromHAR`).

use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::events::PageEvent;
use crate::application::network::Request;
use crate::application::page::Page;
use crate::application::route::Route;
use crate::domain::error::{Error, Result};
use crate::domain::har::{
    Content, Creator, Entry, Har, HarNotFound, HarPage, HarRequest, HarResponse, Log,
    NameValuePair, PostData, RecordHar, iso_time, query_string, request_cookies, response_cookie,
    timings,
};
use crate::domain::url_matcher::UrlMatcher;

/// A HAR being recorded for a context.
pub(crate) struct HarRecording {
    options: RecordHar,
    filter: Option<UrlMatcher>,
    pages: Vec<HarPage>,
    pending: Vec<tokio::task::JoinHandle<Option<Entry>>>,
}

pub(crate) type HarState = Mutex<Option<HarRecording>>;

pub(crate) fn recording(options: &Option<RecordHar>) -> HarState {
    Mutex::new(options.as_ref().map(|o| HarRecording {
        filter: o.url_filter.as_deref().map(UrlMatcher::glob),
        options: o.clone(),
        pages: Vec::new(),
        pending: Vec::new(),
    }))
}

fn pairs(entries: Vec<crate::domain::network::HeaderEntry>) -> Vec<NameValuePair> {
    entries
        .into_iter()
        .map(|h| NameValuePair::new(h.name, h.value))
        .collect()
}

async fn entry_for(request: Request, page_id: String, omit_content: bool) -> Entry {
    let timing = request.timing();
    let request_headers = pairs(request.headers_array().await.unwrap_or_default());
    let cookies = request_headers
        .iter()
        .filter(|h| h.name.eq_ignore_ascii_case("cookie"))
        .flat_map(|h| request_cookies(&h.value))
        .collect();
    let mime = request
        .headers()
        .get("content-type")
        .cloned()
        .unwrap_or_default();
    let post = request.post_data_buffer();
    let mut entry = Entry {
        pageref: Some(page_id),
        started_date_time: iso_time(timing.start_time),
        request: HarRequest {
            method: request.method().to_owned(),
            url: request.url().to_owned(),
            cookies,
            headers: request_headers,
            query_string: query_string(request.url()),
            body_size: post.as_ref().map_or(0, |p| p.len() as i64),
            post_data: post.map(|p| PostData {
                mime_type: mime,
                text: String::from_utf8_lossy(&p).into_owned(),
                params: Vec::new(),
            }),
            ..Default::default()
        },
        resource_type: Some(request.resource_type().to_owned()),
        ..Default::default()
    };
    if let Some(response) = request.existing_response() {
        let headers = pairs(response.headers_array().await.unwrap_or_default());
        let mime = response
            .headers
            .get("content-type")
            .cloned()
            .unwrap_or_else(|| "x-unknown".into());
        let mut content = Content {
            mime_type: mime.clone(),
            ..Default::default()
        };
        if !omit_content
            && !(300..400).contains(&response.status)
            && let Ok(body) = response.body().await
        {
            content = Content::from_body(&body, &mime);
        }
        let redirect_url = headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("location"))
            .map(|h| h.value.clone())
            .unwrap_or_default();
        entry.response = HarResponse {
            status: response.status as i64,
            status_text: response.status_text.clone(),
            http_version: response.http_version().await.unwrap_or_default(),
            cookies: headers
                .iter()
                .filter(|h| h.name.eq_ignore_ascii_case("set-cookie"))
                .filter_map(|h| response_cookie(&h.value))
                .collect(),
            headers,
            content,
            redirect_url,
            ..Default::default()
        };
        if let Some(addr) = response.server_addr() {
            entry.server_ip_address = Some(addr.ip_address);
            entry.server_port = Some(addr.port);
        }
        entry.timings = timings(&request.timing());
        entry.time = entry.timings.total();
    }
    entry
}

impl ContextInner {
    /// Starts recording a page's traffic, when the context records a HAR.
    pub(crate) fn har_track_page(self: &Arc<Self>, page: &Page) {
        let page_id = format!("page@{}", page.target_id());
        {
            let mut har = self.har.lock().unwrap();
            let Some(rec) = har.as_mut() else {
                return;
            };
            rec.pages.push(HarPage {
                id: page_id.clone(),
                ..Default::default()
            });
        }
        let mut events = page.events();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            while let Some(e) = events.next().await {
                let request = match e {
                    PageEvent::RequestFinished(r) | PageEvent::RequestFailed(r) => r,
                    PageEvent::Close => break,
                    _ => continue,
                };
                let Some(ctx) = weak.upgrade() else { break };
                let mut har = ctx.har.lock().unwrap();
                let Some(rec) = har.as_mut() else { break };
                if rec
                    .filter
                    .as_ref()
                    .is_some_and(|f| !f.matches(request.url()))
                {
                    continue;
                }
                let omit = rec.options.omit_content;
                let id = page_id.clone();
                rec.pending.push(tokio::spawn(async move {
                    Some(entry_for(request, id, omit).await)
                }));
            }
        });
    }
}

impl BrowserContext {
    /// Writes the recorded HAR (on close). Does nothing without `record_har`.
    pub(crate) async fn flush_har(&self) -> Result<()> {
        let Some(mut rec) = self.inner.har.lock().unwrap().take() else {
            return Ok(());
        };
        let mut entries = Vec::new();
        for handle in rec.pending.drain(..) {
            if let Ok(Some(e)) = handle.await {
                entries.push(e);
            }
        }
        entries.sort_by(|a, b| a.started_date_time.cmp(&b.started_date_time));
        for page in &mut rec.pages {
            let first = entries
                .iter()
                .find(|e| e.pageref.as_deref() == Some(page.id.as_str()))
                .map(|e| e.started_date_time.clone());
            page.started_date_time = first.unwrap_or_else(|| iso_time(0.0));
        }
        if let Some(open) = self
            .pages()
            .into_iter()
            .find(|p| !p.is_closed())
            .filter(|_| rec.pages.len() == 1)
            && let Ok(title) = open.title().await
        {
            rec.pages[0].title = title;
        }
        let version = self.inner.shared.browser_version.clone();
        let har = Har {
            log: Log {
                browser: Some(Creator {
                    name: "chromium".into(),
                    version,
                }),
                pages: rec.pages,
                entries,
                ..Default::default()
            },
        };
        let json = serde_json::to_vec_pretty(&har)?;
        self.inner
            .shared
            .services
            .fs
            .write(&rec.options.path, &json)?;
        Ok(())
    }
}

/// A pending `routeFromHAR`.
#[must_use = "does nothing until awaited"]
pub struct RouteFromHar<'a> {
    page: Option<&'a Page>,
    context: Option<&'a BrowserContext>,
    path: PathBuf,
    url: Option<UrlMatcher>,
    not_found: HarNotFound,
}

impl<'a> RouteFromHar<'a> {
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
}

async fn serve(route: Route, log: Arc<Log>, not_found: HarNotFound) {
    let request = route.request();
    let post = request.post_data_buffer();
    let entry = log
        .find(request.method(), request.url(), post.as_deref())
        .filter(|e| e.response.status >= 0)
        .cloned();
    let _ = match entry {
        Some(e) => {
            let headers: Vec<(String, String)> = e
                .response
                .headers
                .iter()
                .filter(|h| {
                    !h.name.eq_ignore_ascii_case("content-length")
                        && !h.name.eq_ignore_ascii_case("content-encoding")
                })
                .map(|h| (h.name.clone(), h.value.clone()))
                .collect();
            route
                .fulfill()
                .status(e.response.status as u16)
                .headers(headers)
                .body(e.response.content.body())
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
            let fs = match (self.page, self.context) {
                (Some(p), _) => p.inner.shared.services.fs.clone(),
                (_, Some(c)) => c.inner.shared.services.fs.clone(),
                _ => unreachable!("RouteFromHar always has a target"),
            };
            let bytes = fs.read(&self.path)?;
            let har: Har = serde_json::from_slice(&bytes).map_err(|e| {
                Error::InvalidArgument(format!(
                    "Error reading HAR file {}: {e}",
                    self.path.display()
                ))
            })?;
            let log = Arc::new(har.log);
            let url = self.url.unwrap_or_else(|| UrlMatcher::glob("**/*"));
            let not_found = self.not_found;
            let handler = move |r: Route| {
                let log = log.clone();
                async move { serve(r, log, not_found).await }
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

impl Page {
    /// `page.routeFromHAR(path)`: answers matching requests from a HAR file.
    pub fn route_from_har(&self, path: impl Into<PathBuf>) -> RouteFromHar<'_> {
        RouteFromHar {
            page: Some(self),
            context: None,
            path: path.into(),
            url: None,
            not_found: HarNotFound::Abort,
        }
    }
}

impl BrowserContext {
    /// `context.routeFromHAR(path)`.
    pub fn route_from_har(&self, path: impl Into<PathBuf>) -> RouteFromHar<'_> {
        RouteFromHar {
            page: None,
            context: Some(self),
            path: path.into(),
            url: None,
            not_found: HarNotFound::Abort,
        }
    }
}
