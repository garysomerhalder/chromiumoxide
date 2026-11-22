//! Smaller Playwright APIs: default timeouts, `emulateMedia`, locator
//! handlers, `waitForNavigation`, CDP sessions, Chrome tracing.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use base64::Engine;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde_json::{Value, json};

use crate::application::browser::Browser;
use crate::application::context::BrowserContext;
use crate::application::locator::Locator;
use crate::application::page::{Page, Response};
use crate::application::page_init::emulated_media_params;
use crate::application::wait::wait_until;
use crate::domain::emulation::{ColorScheme, Contrast, ForcedColors, Media, ReducedMotion};
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::url_matcher::UrlMatcher;
use crate::ports::CdpSession;

/// `page.emulateMedia(options)`: fields left `None` keep their current value.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EmulateMedia {
    pub media: Option<Media>,
    pub color_scheme: Option<ColorScheme>,
    pub reduced_motion: Option<ReducedMotion>,
    pub forced_colors: Option<ForcedColors>,
    pub contrast: Option<Contrast>,
}

type HandlerFn = Arc<dyn Fn(Locator) -> BoxFuture<'static, ()> + Send + Sync>;

pub(crate) struct LocatorHandler {
    locator: Locator,
    handler: HandlerFn,
    times_left: Option<usize>,
    no_wait_after: bool,
}

impl Page {
    /// The context this page belongs to.
    pub fn context(&self) -> Option<BrowserContext> {
        self.inner
            .context
            .upgrade()
            .map(|inner| BrowserContext { inner })
    }

    /// `page.setDefaultNavigationTimeout(timeout)`.
    pub fn set_default_navigation_timeout(&self, timeout: Duration) {
        *self.inner.navigation_timeout.lock().unwrap() = Some(timeout);
    }

    /// The timeout for navigations: the navigation default, else the default.
    pub(crate) fn navigation_timeout(&self) -> Duration {
        self.inner
            .navigation_timeout
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.default_timeout())
    }

    /// `page.bringToFront()`.
    pub async fn bring_to_front(&self) -> Result<()> {
        self.send("Page.bringToFront", json!({})).await?;
        Ok(())
    }

    /// `page.emulateMedia(options)`.
    pub async fn emulate_media(&self, options: EmulateMedia) -> Result<()> {
        let merged = {
            let mut m = self.inner.media.lock().unwrap();
            if options.media.is_some() {
                m.media = options.media;
            }
            if options.color_scheme.is_some() {
                m.color_scheme = options.color_scheme;
            }
            if options.reduced_motion.is_some() {
                m.reduced_motion = options.reduced_motion;
            }
            if options.forced_colors.is_some() {
                m.forced_colors = options.forced_colors;
            }
            if options.contrast.is_some() {
                m.contrast = options.contrast;
            }
            *m
        };
        let mut o = (*self.inner.options).clone();
        o.color_scheme = merged.color_scheme.or(o.color_scheme);
        o.reduced_motion = merged.reduced_motion.or(o.reduced_motion);
        o.forced_colors = merged.forced_colors.or(o.forced_colors);
        o.contrast = merged.contrast.or(o.contrast);
        let params = emulated_media_params(&o, merged.media.unwrap_or(Media::NoOverride));
        self.send("Emulation.setEmulatedMedia", params).await?;
        Ok(())
    }

    /// `page.addLocatorHandler(locator, handler)`: before an action, when
    /// `locator` is visible, `handler` runs (e.g. to dismiss an overlay).
    pub async fn add_locator_handler<F, Fut>(&self, locator: &Locator, handler: F) -> Result<()>
    where
        F: Fn(Locator) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.add_locator_handler_with(locator, handler, None, false)
            .await
    }

    /// `addLocatorHandler` with `times` and `noWaitAfter`.
    pub async fn add_locator_handler_with<F, Fut>(
        &self,
        locator: &Locator,
        handler: F,
        times: Option<usize>,
        no_wait_after: bool,
    ) -> Result<()>
    where
        F: Fn(Locator) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.inner
            .locator_handlers
            .lock()
            .unwrap()
            .push(LocatorHandler {
                locator: locator.clone(),
                handler: Arc::new(move |l| handler(l).boxed()),
                times_left: times,
                no_wait_after,
            });
        Ok(())
    }

    /// `page.removeLocatorHandler(locator)`.
    pub async fn remove_locator_handler(&self, locator: &Locator) -> Result<()> {
        self.inner
            .locator_handlers
            .lock()
            .unwrap()
            .retain(|h| h.locator.selector() != locator.selector());
        Ok(())
    }

    /// Runs the locator handlers whose locator is visible (before an action).
    pub(crate) async fn run_locator_handlers(&self) -> Result<()> {
        if self.inner.handling_locator.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = async {
            let handlers: Vec<(String, Locator, HandlerFn, bool)> = self
                .inner
                .locator_handlers
                .lock()
                .unwrap()
                .iter()
                .map(|h| {
                    (
                        h.locator.selector().to_owned(),
                        h.locator.clone(),
                        h.handler.clone(),
                        h.no_wait_after,
                    )
                })
                .collect();
            for (key, locator, handler, no_wait_after) in handlers {
                if !locator.is_visible().await.unwrap_or(false) {
                    continue;
                }
                handler(locator.clone()).await;
                {
                    let mut list = self.inner.locator_handlers.lock().unwrap();
                    if let Some(h) = list.iter_mut().find(|h| h.locator.selector() == key)
                        && let Some(n) = h.times_left.as_mut()
                    {
                        *n = n.saturating_sub(1);
                    }
                    list.retain(|h| h.times_left != Some(0));
                }
                if !no_wait_after {
                    let clock = self.inner.shared.services.clock.clone();
                    let deadline = clock.now_ms() + self.default_timeout().as_millis() as u64;
                    while locator.is_visible().await.unwrap_or(false) {
                        if clock.now_ms() > deadline {
                            break;
                        }
                        clock.sleep(Duration::from_millis(50)).await;
                    }
                }
            }
            Ok(())
        }
        .await;
        self.inner.handling_locator.store(false, Ordering::SeqCst);
        result
    }

    /// `page.waitForNavigation()` (deprecated in Playwright, still supported).
    pub fn wait_for_navigation(&self) -> WaitForNavigation<'_> {
        WaitForNavigation {
            page: self,
            frame: None,
            url: None,
            wait_until: LifecycleEvent::Load,
            timeout: None,
        }
    }
}

/// A pending `page.waitForNavigation()`.
#[must_use = "does nothing until awaited"]
pub struct WaitForNavigation<'a> {
    page: &'a Page,
    /// `None`: the main frame.
    frame: Option<String>,
    url: Option<UrlMatcher>,
    wait_until: LifecycleEvent,
    timeout: Option<Duration>,
}

impl<'a> WaitForNavigation<'a> {
    /// Waits for `frame_id`'s navigation instead of the main frame's.
    pub(crate) fn for_frame(mut self, frame_id: &str) -> Self {
        self.frame = Some(frame_id.to_owned());
        self
    }
    pub fn url(mut self, url: impl Into<UrlMatcher>) -> Self {
        self.url = Some(url.into());
        self
    }
    pub fn wait_until(mut self, event: LifecycleEvent) -> Self {
        self.wait_until = event;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The waited frame's navigation count.
    fn count(&self) -> u64 {
        let st = self.page.inner.state.lock().unwrap();
        match &self.frame {
            None => st.main_navigations,
            Some(id) => st.frame_navigations.get(id).map_or(0, |n| n.0),
        }
    }

    async fn wait_after(self, before: u64) -> Result<Option<Response>> {
        let page = self.page;
        let timeout = self.timeout.unwrap_or_else(|| page.navigation_timeout());
        let inner = &page.inner;
        let url = self.url;
        let wait = self.wait_until;
        let frame_id = self.frame;
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            timeout,
            "waiting for navigation",
            || {
                if inner.closed.load(Ordering::SeqCst) {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                let (count, same_document) = match &frame_id {
                    None => (st.main_navigations, st.last_navigation_same_document),
                    Some(id) => st.frame_navigations.get(id).copied().unwrap_or_default(),
                };
                if count == before {
                    return None;
                }
                let frame = match &frame_id {
                    None => st.tree.main_frame()?,
                    Some(id) => match st.tree.frame(id) {
                        Some(f) => f,
                        None => {
                            return Some(Err(Error::TargetClosed(
                                "frame has been detached".into(),
                            )));
                        }
                    },
                };
                if url.as_ref().is_some_and(|m| !m.matches(&frame.url)) {
                    return None;
                }
                if same_document {
                    return Some(Ok(None));
                }
                if !frame.has(wait) {
                    return None;
                }
                Some(Ok(frame
                    .loader_id
                    .as_ref()
                    .and_then(|l| st.responses.get(l))
                    .cloned()))
            },
        )
        .await
    }

    /// Starts waiting, runs `action`, then waits for the navigation.
    pub async fn run<A, R>(self, action: A) -> Result<Option<Response>>
    where
        A: IntoFuture<Output = Result<R>>,
    {
        let before = self.count();
        action.await?;
        self.wait_after(before).await
    }
}

impl<'a> IntoFuture for WaitForNavigation<'a> {
    type Output = Result<Option<Response>>;
    type IntoFuture = BoxFuture<'a, Result<Option<Response>>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let before = self.count();
            self.wait_after(before).await
        }
        .boxed()
    }
}

impl BrowserContext {
    /// `context.backgroundPages()`: deprecated in Playwright, which always
    /// returns an empty list (Manifest V3 extensions have no background pages).
    #[deprecated(note = "always empty, like Playwright's")]
    pub fn background_pages(&self) -> Vec<Page> {
        Vec::new()
    }

    /// The browser this context belongs to.
    pub fn browser(&self) -> Option<Browser> {
        self.inner.shared.browser()
    }

    /// `context.setDefaultTimeout(timeout)`: for its pages, now and later.
    pub fn set_default_timeout(&self, timeout: Duration) {
        *self.inner.default_timeout.lock().unwrap() = Some(timeout);
        for page in self.pages() {
            page.set_default_timeout(timeout);
        }
    }

    /// `context.setDefaultNavigationTimeout(timeout)`.
    pub fn set_default_navigation_timeout(&self, timeout: Duration) {
        *self.inner.navigation_timeout.lock().unwrap() = Some(timeout);
        for page in self.pages() {
            page.set_default_navigation_timeout(timeout);
        }
    }

    /// `context.newCDPSession(page)`: raw protocol access to a page.
    pub async fn new_cdp_session(&self, page: &Page) -> Result<CdpSessionHandle> {
        Ok(CdpSessionHandle {
            cdp: page.inner.shared.cdp.clone(),
            session_id: Some(page.inner.session_id.clone()),
        })
    }
}

/// A raw protocol session (Playwright's `CDPSession`).
#[derive(Clone)]
pub struct CdpSessionHandle {
    cdp: Arc<dyn CdpSession>,
    session_id: Option<String>,
}

impl std::fmt::Debug for CdpSessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CdpSession")
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl CdpSessionHandle {
    /// `session.send(method, params)`.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.cdp
            .call(self.session_id.as_deref(), method, params)
            .await
    }

    /// Protocol events of this session from now on.
    pub fn events(
        &self,
    ) -> impl futures::Stream<Item = Arc<crate::ports::RawEvent>> + Send + use<> {
        let sid = self.session_id.clone();
        self.cdp
            .subscribe()
            .filter(move |e| futures::future::ready(e.session_id == sid))
    }

    /// `session.detach()`: the session is shared with chromiumoxide, so this
    /// only drops this handle's use of it.
    pub async fn detach(&self) -> Result<()> {
        Ok(())
    }
}

/// `browser.browserType()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserType;

impl BrowserType {
    pub fn name(&self) -> &'static str {
        "chromium"
    }
}

/// Playwright's default Chrome trace categories.
const TRACE_CATEGORIES: &[&str] = &[
    "-*",
    "devtools.timeline",
    "v8.execute",
    "disabled-by-default-devtools.timeline",
    "disabled-by-default-devtools.timeline.frame",
    "toplevel",
    "blink.console",
    "blink.user_timing",
    "latencyInfo",
    "disabled-by-default-devtools.timeline.stack",
    "disabled-by-default-v8.cpu_profiler",
    "disabled-by-default-v8.cpu_profiler.hires",
];

impl Browser {
    /// `browser.browserType()`.
    pub fn browser_type(&self) -> BrowserType {
        BrowserType
    }

    /// `browser.newBrowserCDPSession()`.
    pub async fn new_browser_cdp_session(&self) -> Result<CdpSessionHandle> {
        Ok(CdpSessionHandle {
            cdp: self.cdp(),
            session_id: None,
        })
    }

    /// `browser.startTracing(page, {categories})`: Chrome performance tracing.
    pub async fn start_tracing(&self, page: Option<&Page>, categories: &[&str]) -> Result<()> {
        let cats: Vec<&str> = if categories.is_empty() {
            TRACE_CATEGORIES.to_vec()
        } else {
            categories.to_vec()
        };
        let sid = page.map(|p| p.inner.session_id.clone());
        *self.tracing_session().lock().unwrap() = Some(sid.clone());
        self.cdp()
            .call(
                sid.as_deref(),
                "Tracing.start",
                json!({"transferMode": "ReturnAsStream", "categories": cats.join(",")}),
            )
            .await?;
        Ok(())
    }

    /// `browser.stopTracing()`: the trace as Chrome's JSON.
    pub async fn stop_tracing(&self) -> Result<Vec<u8>> {
        let sid = self
            .tracing_session()
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
        let cdp = self.cdp();
        let mut events = cdp.subscribe();
        cdp.call(sid.as_deref(), "Tracing.end", json!({})).await?;
        let stream = loop {
            let Some(e) = events.next().await else {
                return Err(Error::TargetClosed("browser closed while tracing".into()));
            };
            if e.method == "Tracing.tracingComplete" && e.session_id == sid {
                break e
                    .params
                    .get("stream")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
        };
        let handle = stream.ok_or_else(|| Error::Serde("tracing returned no stream".into()))?;
        let mut out = Vec::new();
        loop {
            let chunk = cdp
                .call(sid.as_deref(), "IO.read", json!({"handle": handle}))
                .await?;
            let data = chunk
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if chunk.get("base64Encoded").and_then(Value::as_bool) == Some(true) {
                out.extend(
                    base64::engine::general_purpose::STANDARD
                        .decode(data)
                        .map_err(|e| Error::Serde(e.to_string()))?,
                );
            } else {
                out.extend_from_slice(data.as_bytes());
            }
            if chunk.get("eof").and_then(Value::as_bool) == Some(true) {
                break;
            }
        }
        let _ = cdp
            .call(sid.as_deref(), "IO.close", json!({"handle": handle}))
            .await;
        Ok(out)
    }
}
