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
    Content, Creator, Entry, Har, HarContent, HarMode, HarNotFound, HarPage, HarRequest,
    HarResponse, Log, NameValuePair, PostData, RecordHar, attachment_name, iso_time, query_string,
    request_cookies, response_cookie, slim, timings,
};
use crate::domain::url_matcher::UrlMatcher;

/// A HAR being recorded for a context.
pub(crate) struct HarRecording {
    options: RecordHar,
    filter: Option<UrlMatcher>,
    pages: Vec<HarPage>,
    pending: Vec<tokio::task::JoinHandle<Option<RecordedEntry>>>,
    /// Only this page's traffic (`page.routeFromHAR(.., { update })`).
    page_only: Option<String>,
}

/// An attached body: its file name and bytes.
pub(crate) type Attachment = (String, Vec<u8>);

/// A recorded entry and its attached body.
pub(crate) type RecordedEntry = (Entry, Option<Attachment>);

impl HarRecording {
    pub(crate) fn new(
        options: RecordHar,
        filter: Option<UrlMatcher>,
        page_only: Option<String>,
    ) -> Self {
        HarRecording {
            filter: filter.or_else(|| options.url_filter.as_deref().map(UrlMatcher::glob)),
            options,
            pages: Vec::new(),
            pending: Vec::new(),
            page_only,
        }
    }
}

pub(crate) type HarState = Mutex<Option<HarRecording>>;

pub(crate) fn recording(options: &Option<RecordHar>) -> HarState {
    Mutex::new(
        options
            .as_ref()
            .map(|o| HarRecording::new(o.clone(), None, None)),
    )
}

fn pairs(entries: Vec<crate::domain::network::HeaderEntry>) -> Vec<NameValuePair> {
    entries
        .into_iter()
        .map(|h| NameValuePair::new(h.name, h.value))
        .collect()
}

pub(crate) async fn entry_for(
    request: Request,
    page_id: String,
    content_policy: HarContent,
) -> (Entry, Option<Attachment>) {
    let mut attachment = None;
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
        if content_policy != HarContent::Omit
            && !(300..400).contains(&response.status)
            && let Ok(body) = response.body().await
        {
            content = Content::from_body(&body, &mime);
            if content_policy == HarContent::Attach {
                let name = attachment_name(&body, &mime);
                content.text = None;
                content.encoding = None;
                content.file = Some(name.clone());
                attachment = Some((name, body));
            }
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
    (entry, attachment)
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
            if rec.page_only.as_ref().is_some_and(|p| *p != page_id) {
                return;
            }
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
                let policy = rec.options.content_policy();
                let id = page_id.clone();
                rec.pending.push(tokio::spawn(async move {
                    Some(entry_for(request, id, policy).await)
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
        let mut attachments: Vec<Attachment> = Vec::new();
        for handle in rec.pending.drain(..) {
            if let Ok(Some((e, a))) = handle.await {
                entries.push(e);
                if let Some(a) = a.filter(|a| !attachments.iter().any(|b| b.0 == a.0)) {
                    attachments.push(a);
                }
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
        if rec.options.mode == HarMode::Minimal {
            rec.pages.clear();
            entries.iter_mut().for_each(slim);
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
        write_har(
            &*self.inner.shared.services.fs,
            &rec.options,
            &har,
            &attachments,
        )
    }
}

/// Writes a recorded HAR: as a zip with `har.har` and the attached bodies,
/// or as a `.har` with the bodies next to it (or in `resources_dir`).
pub(crate) fn write_har(
    fs: &dyn crate::ports::FileSystem,
    options: &RecordHar,
    har: &Har,
    attachments: &[Attachment],
) -> Result<()> {
    let json = serde_json::to_vec_pretty(har)?;
    if options.is_zip() {
        let mut files: Vec<(&str, Vec<u8>)> = vec![("har.har", json)];
        files.extend(attachments.iter().map(|(n, b)| (n.as_str(), b.clone())));
        fs.write(&options.path, &crate::domain::trace::zip_stored(&files))?;
        return Ok(());
    }
    fs.write(&options.path, &json)?;
    let dir = options
        .resources_dir
        .clone()
        .or_else(|| options.path.parent().map(PathBuf::from))
        .unwrap_or_default();
    for (name, body) in attachments {
        fs.write(&dir.join(name), body)?;
    }
    Ok(())
}

/// A pending `routeFromHAR`.
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

async fn serve(route: Route, log: Arc<Log>, not_found: HarNotFound, base: Arc<HarBase>) {
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

/// Where a HAR's attached bodies are read from.
struct HarBase {
    fs: Arc<dyn crate::ports::FileSystem>,
    dir: PathBuf,
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
            if self.update {
                let context = match (self.page, self.context) {
                    (Some(p), _) => {
                        p.context()
                            .ok_or_else(|| {
                                Error::TargetClosed(
                                    "page.routeFromHAR: the page has no context".into(),
                                )
                            })?
                            .inner
                    }
                    (_, Some(c)) => c.inner.clone(),
                    _ => unreachable!("RouteFromHar always has a target"),
                };
                let options = RecordHar {
                    content: Some(self.update_content),
                    mode: self.update_mode,
                    ..RecordHar::new(&self.path)
                };
                let page_only = self.page.map(|p| format!("page@{}", p.target_id()));
                {
                    let mut har = context.har.lock().unwrap();
                    if har.is_some() {
                        return Err(Error::InvalidArgument(
                            "HAR recording is already started".into(),
                        ));
                    }
                    *har = Some(HarRecording::new(options, self.url.clone(), page_only));
                }
                let pages: Vec<Page> = context.pages.lock().unwrap().clone();
                for page in pages {
                    context.har_track_page(&page);
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
                fs: fs.clone(),
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

impl Page {
    /// `page.routeFromHAR(path)`: answers matching requests from a HAR file.
    pub fn route_from_har(&self, path: impl Into<PathBuf>) -> RouteFromHar<'_> {
        RouteFromHar {
            page: Some(self),
            context: None,
            path: path.into(),
            url: None,
            not_found: HarNotFound::Abort,
            update: false,
            update_content: HarContent::Attach,
            update_mode: HarMode::Minimal,
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
            update: false,
            update_content: HarContent::Attach,
            update_mode: HarMode::Minimal,
        }
    }
}
