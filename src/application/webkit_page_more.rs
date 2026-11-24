//! The rest of Playwright's `Page`, `BrowserContext` and `Browser` on the
//! WebKit engine: recent console messages, errors and requests; default
//! timeouts; opener and context; waits; pickLocator; pause; requestGC;
//! emulateMedia; the mouse and keyboard; context settings after creation
//! (extra headers, offline, geolocation, permissions, HTTP credentials,
//! storage state) and context events. And the Locator and FrameLocator
//! extras (filter, and, or, all, boundingBox, screenshot, evaluate, nth,
//! owner). Protocol commands as in Playwright's `WKPage` and
//! `WKBrowserContext`.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::debugger::{PICK_STOP, PICK_TICK};
use crate::application::extras::EmulateMedia;
use crate::application::locator::BoundingBox;
use crate::application::wait::with_timeout;
use crate::application::webkit::{Browser, BrowserContext, ContextInner, Page};
use crate::application::webkit_events::{ConsoleMessage, ContextEvent, PageError, PageEvent};
use crate::application::webkit_frame::FrameLocator;
use crate::application::webkit_locator::Locator;
use crate::application::webkit_network::{Request, Response};
use crate::application::webkit_worker::Worker;
use crate::domain::emulation::{
    ColorScheme, Contrast, ForcedColors, Geolocation, HttpCredentials, Media, ReducedMotion,
};
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::input::MouseButton;
use crate::domain::screenshot::Rect;
use crate::domain::selectors::{Filter, compose};
use crate::domain::storage::{Cookie, NameValue, NewCookie, OriginState, StorageState};
use crate::domain::url_matcher::UrlMatcher;

/// No network activity for this long is `networkidle`.
const IDLE: Duration = Duration::from_millis(500);

/// What a page remembers for `consoleMessages()`, `pageErrors()` and
/// `requests()` (Playwright keeps the last 200, 200 and 100).
#[derive(Default)]
pub(crate) struct History {
    console: VecDeque<ConsoleMessage>,
    errors: VecDeque<PageError>,
    requests: VecDeque<Request>,
}

impl History {
    pub(crate) fn record(&mut self, event: &PageEvent) {
        fn push<T>(q: &mut VecDeque<T>, v: T, max: usize) {
            if q.len() == max {
                q.pop_front();
            }
            q.push_back(v);
        }
        match event {
            PageEvent::Console(m) => push(&mut self.console, m.clone(), 200),
            PageEvent::PageError(e) => push(&mut self.errors, e.clone(), 200),
            PageEvent::Request(r) => push(&mut self.requests, r.clone(), 100),
            _ => {}
        }
    }
}

/// The error for what only Chromium has.
fn chromium_only(what: &str) -> Error {
    Error::InvalidArgument(format!(
        "{what} is only available in Chromium (it needs the Chrome DevTools Protocol)"
    ))
}

impl Page {
    /// `page.context()`: the context the page belongs to.
    pub fn context(&self) -> Option<BrowserContext> {
        self.inner
            .context
            .upgrade()
            .map(|inner| BrowserContext { inner })
    }

    /// `page.opener()`: the page that opened this one as a pop-up.
    pub fn opener(&self) -> Option<Page> {
        self.inner.opener.lock().unwrap().clone()
    }

    /// `page.bringToFront()`: makes the page active and focused.
    pub async fn bring_to_front(&self) -> Result<()> {
        self.proxy_call("Emulation.setActiveAndFocused", json!({"active": true}))
            .await
            .map(drop)
    }

    /// `page.consoleMessages()`: the most recent console messages (up to 200).
    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.inner
            .history
            .lock()
            .unwrap()
            .console
            .iter()
            .cloned()
            .collect()
    }

    /// `page.pageErrors()`: the most recent uncaught errors (up to 200).
    pub fn page_errors(&self) -> Vec<PageError> {
        self.inner
            .history
            .lock()
            .unwrap()
            .errors
            .iter()
            .cloned()
            .collect()
    }

    /// `page.requests()`: the most recent requests (up to 100).
    pub fn requests(&self) -> Vec<Request> {
        self.inner
            .history
            .lock()
            .unwrap()
            .requests
            .iter()
            .cloned()
            .collect()
    }

    /// `page.clearConsoleMessages()`.
    pub fn clear_console_messages(&self) {
        self.inner.history.lock().unwrap().console.clear();
    }

    /// `page.clearPageErrors()`.
    pub fn clear_page_errors(&self) {
        self.inner.history.lock().unwrap().errors.clear();
    }

    /// `page.setDefaultTimeout(timeout)`: for actions and waits.
    pub fn set_default_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().action = Some(timeout);
    }

    /// `page.setDefaultNavigationTimeout(timeout)`.
    pub fn set_default_navigation_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().navigation = Some(timeout);
    }

    /// `page.waitForNavigation()` (deprecated in Playwright: use
    /// `wait_for_url`): the next `load` of the main frame. Listens from the
    /// call on, so start it before the action.
    pub fn wait_for_navigation(&self) -> impl Future<Output = Result<()>> + Send + use<> {
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        let timeout = self.navigation_timeout();
        async move {
            with_timeout(&*clock, timeout, "waiting for navigation", async {
                while let Some(e) = events.next().await {
                    match e {
                        PageEvent::Load => return Ok(()),
                        PageEvent::Close => break,
                        _ => {}
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }

    /// `page.waitForURL(url)`: until the main frame's URL matches, then its
    /// `load`.
    pub async fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let matcher = url.into();
        let timeout = self.navigation_timeout();
        let deadline = self.now() + timeout;
        while !matcher.matches(&self.url()) {
            if self.now() >= deadline {
                return Err(Error::Timeout {
                    ms: timeout.as_millis() as u64,
                    action: format!("waiting for URL, now {}", self.url()),
                });
            }
            self.sleep(Duration::from_millis(16)).await;
        }
        self.wait_for_load_state(LifecycleEvent::Load).await
    }

    /// `page.waitForLoadState(state)`: `domcontentloaded` and `load` from
    /// the document's `readyState`; `networkidle` is `load` and then no
    /// request, response or finish for 500 ms.
    pub async fn wait_for_load_state(&self, state: LifecycleEvent) -> Result<()> {
        let ready = match state {
            LifecycleEvent::Commit => return Ok(()),
            LifecycleEvent::DomContentLoaded => "document.readyState !== 'loading'",
            LifecycleEvent::Load | LifecycleEvent::NetworkIdle => {
                "document.readyState === 'complete'"
            }
        };
        let mut events = self.events();
        self.wait_for_function(ready).await?;
        if state != LifecycleEvent::NetworkIdle {
            return Ok(());
        }
        let clock = self.inner.services.clock.clone();
        let quiet = async {
            loop {
                let busy = with_timeout(&*clock, IDLE, "network idle", async {
                    loop {
                        match events.next().await {
                            Some(
                                PageEvent::Request(_)
                                | PageEvent::Response(_)
                                | PageEvent::RequestFinished(_)
                                | PageEvent::RequestFailed(_),
                            ) => return Ok(()),
                            Some(PageEvent::Close) | None => {
                                return Err(Error::TargetClosed("page closed".into()));
                            }
                            Some(_) => {}
                        }
                    }
                })
                .await;
                match busy {
                    Ok(()) => continue,
                    Err(Error::Timeout { .. }) => return Ok(()),
                    Err(e) => return Err(e),
                }
            }
        };
        with_timeout(&*clock, self.timeout(), "waiting for networkidle", quiet).await
    }

    /// `page.waitForRequest(url)`: the next request to a matching URL.
    /// Listens from the call on, so start it before the action.
    pub fn wait_for_request(
        &self,
        url: impl Into<UrlMatcher>,
    ) -> impl Future<Output = Result<Request>> + Send + 'static {
        let matcher = url.into();
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        let timeout = self.timeout();
        async move {
            with_timeout(&*clock, timeout, "waiting for request", async {
                while let Some(e) = events.next().await {
                    if let PageEvent::Request(r) = e
                        && matcher.matches(r.url())
                    {
                        return Ok(r);
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }

    /// `page.waitForResponse(url)`: the next response from a matching URL.
    pub fn wait_for_response(
        &self,
        url: impl Into<UrlMatcher>,
    ) -> impl Future<Output = Result<Response>> + Send + 'static {
        let matcher = url.into();
        let mut events = self.events();
        let clock = self.inner.services.clock.clone();
        let timeout = self.timeout();
        async move {
            with_timeout(&*clock, timeout, "waiting for response", async {
                while let Some(e) = events.next().await {
                    if let PageEvent::Response(r) = e
                        && matcher.matches(r.url())
                    {
                        return Ok(r);
                    }
                }
                Err(Error::TargetClosed("page closed".into()))
            })
            .await
        }
    }

    /// `page.pickLocator()`: waits for the user to click an element (in any
    /// frame) and returns its locator. Hovering highlights elements.
    pub async fn pick_locator(&self) -> Result<Locator> {
        self.inner.picking.store(true, Ordering::SeqCst);
        let result = loop {
            if !self.inner.picking.load(Ordering::SeqCst) || self.is_closed() {
                break Err(Error::InvalidArgument(
                    "Locator picking was cancelled".into(),
                ));
            }
            let mut picked = None;
            for frame in self.frames().await.unwrap_or_default() {
                if let Ok(Value::String(selector)) =
                    self.injected_call_in(frame.scope(), PICK_TICK, &[]).await
                {
                    picked = Some(frame.locator(&selector));
                    break;
                }
            }
            if let Some(locator) = picked {
                break Ok(locator);
            }
            self.sleep(Duration::from_millis(100)).await;
        };
        self.inner.picking.store(false, Ordering::SeqCst);
        for frame in self.frames().await.unwrap_or_default() {
            let _ = self
                .call_json_frame(PICK_STOP, &[], frame.scope(), true)
                .await;
        }
        result
    }

    /// `page.cancelPickLocator()`: leaves picking mode (no-op otherwise).
    pub async fn cancel_pick_locator(&self) -> Result<()> {
        self.inner.picking.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// `page.pause()`: waits until `context.resume()` (Playwright's
    /// inspector "Resume").
    pub async fn pause(&self) -> Result<()> {
        let Some(ctx) = self.inner.context.upgrade() else {
            return Ok(());
        };
        ctx.paused.send_replace(true);
        let mut rx = ctx.paused.subscribe();
        while *rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
        Ok(())
    }

    /// `page.requestGC()` (`Heap.gc`).
    pub async fn request_gc(&self) -> Result<()> {
        self.target_call("Heap.gc", json!({})).await.map(drop)
    }

    /// `page.emulateMedia(...)`: media type, color scheme, reduced motion,
    /// forced colors and contrast (`Page.setEmulatedMedia`,
    /// `Page.overrideUserPreference`, `Page.setForcedColors`). Options left
    /// `None` keep their current value.
    pub async fn emulate_media(&self, options: EmulateMedia) -> Result<()> {
        {
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
        }
        for (method, params) in self.media_commands() {
            self.target_call(method, params).await?;
        }
        Ok(())
    }

    /// The commands that set the page's media emulation on a target: the
    /// context's options, overridden by `emulate_media`.
    pub(crate) fn media_commands(&self) -> Vec<(&'static str, Value)> {
        let o = &self.inner.options;
        let m = *self.inner.media.lock().unwrap();
        let mut out = Vec::new();
        if let Some(media) = m.media {
            let media = match media {
                Media::Screen => "screen",
                Media::Print => "print",
                Media::NoOverride => "",
            };
            out.push(("Page.setEmulatedMedia", json!({"media": media})));
        }
        let pref = |name: &str, value: Option<&str>| {
            let mut p = json!({"name": name});
            if let Some(v) = value {
                p["value"] = json!(v);
            }
            ("Page.overrideUserPreference", p)
        };
        if let Some(c) = m.color_scheme.or(o.color_scheme) {
            out.push(pref(
                "PrefersColorScheme",
                match c {
                    ColorScheme::Light => Some("Light"),
                    ColorScheme::Dark => Some("Dark"),
                    ColorScheme::NoPreference => Some("NoPreference"),
                    ColorScheme::NoOverride => None,
                },
            ));
        }
        if let Some(r) = m.reduced_motion.or(o.reduced_motion) {
            out.push(pref(
                "PrefersReducedMotion",
                match r {
                    ReducedMotion::Reduce => Some("Reduce"),
                    ReducedMotion::NoPreference => Some("NoPreference"),
                    ReducedMotion::NoOverride => None,
                },
            ));
        }
        if let Some(c) = m.contrast.or(o.contrast) {
            out.push(pref(
                "PrefersContrast",
                match c {
                    Contrast::More => Some("More"),
                    Contrast::NoPreference => Some("NoPreference"),
                    Contrast::NoOverride => None,
                },
            ));
        }
        if let Some(f) = m.forced_colors.or(o.forced_colors) {
            out.push((
                "Page.setForcedColors",
                match f {
                    ForcedColors::Active => json!({"forcedColors": "Active"}),
                    ForcedColors::None => json!({"forcedColors": "None"}),
                    ForcedColors::NoOverride => json!({}),
                },
            ));
        }
        out
    }

    /// The context's extra headers, then the page's own.
    pub(crate) fn merged_headers(&self) -> serde_json::Map<String, Value> {
        let mut merged: Vec<(String, String)> = self
            .inner
            .context
            .upgrade()
            .map(|c| c.extra_headers.lock().unwrap().clone())
            .unwrap_or_default();
        let own = self.inner.extra_headers.lock().unwrap().clone();
        merged.retain(|(k, _)| !own.iter().any(|(o, _)| o.eq_ignore_ascii_case(k)));
        merged.extend(own);
        merged
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect()
    }

    /// `page.setExtraHTTPHeaders(headers)`: added to the context's for this
    /// page's requests.
    pub async fn set_extra_http_headers(&self, headers: &[(&str, &str)]) -> Result<()> {
        *self.inner.extra_headers.lock().unwrap() = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        self.apply_headers().await
    }

    pub(crate) async fn apply_headers(&self) -> Result<()> {
        self.target_call(
            "Network.setExtraHTTPHeaders",
            json!({"headers": self.merged_headers()}),
        )
        .await
        .map(drop)
    }

    /// Whether the context is offline (`context.setOffline`).
    pub(crate) fn offline(&self) -> bool {
        self.inner
            .context
            .upgrade()
            .is_some_and(|c| c.offline.load(Ordering::SeqCst))
    }

    /// The context's permissions and HTTP credentials, on this page's
    /// proxy.
    pub(crate) async fn apply_context_settings(&self) -> Result<()> {
        let Some(ctx) = self.inner.context.upgrade() else {
            return Ok(());
        };
        let granted = ctx.granted.lock().unwrap().clone();
        for (origin, permissions) in granted {
            self.proxy_call(
                "Emulation.grantPermissions",
                json!({"origin": origin, "permissions": permissions}),
            )
            .await?;
        }
        let credentials = ctx.credentials.lock().unwrap().clone();
        if credentials.is_some() {
            self.apply_credentials(credentials.as_ref()).await?;
        }
        Ok(())
    }

    async fn apply_credentials(&self, credentials: Option<&HttpCredentials>) -> Result<()> {
        let list: Vec<Value> = credentials
            .map(|c| {
                let mut v = json!({"username": c.username, "password": c.password});
                if let Some(o) = &c.origin {
                    v["origin"] = json!(o);
                }
                v
            })
            .into_iter()
            .collect();
        self.proxy_call("Emulation.setAuthCredentials", json!({"credentials": list}))
            .await
            .map(drop)
    }

    /// `page.mouse()` (viewport CSS pixels).
    pub fn mouse(&self) -> Mouse {
        Mouse { page: self.clone() }
    }

    /// `page.keyboard()`.
    pub fn keyboard(&self) -> Keyboard {
        Keyboard { page: self.clone() }
    }
}

/// `page.mouse()` on WebKit (`Input.dispatchMouseEvent`).
#[derive(Clone, Debug)]
pub struct Mouse {
    page: Page,
}

impl Mouse {
    /// `mouse.move(x, y, { steps })`.
    pub async fn move_to(&self, x: f64, y: f64, steps: u32) -> Result<()> {
        let moves = self.page.inner.mouse.lock().unwrap().move_to(x, y, steps);
        for m in moves {
            self.page.dispatch_mouse(m).await?;
        }
        Ok(())
    }

    /// `mouse.down({ button })`.
    pub async fn down(&self, button: MouseButton) -> Result<()> {
        let e = self.page.inner.mouse.lock().unwrap().down(button, 1);
        self.page.dispatch_mouse(e).await
    }

    /// `mouse.up({ button })`.
    pub async fn up(&self, button: MouseButton) -> Result<()> {
        let e = self.page.inner.mouse.lock().unwrap().up(button, 1);
        self.page.dispatch_mouse(e).await
    }

    /// `mouse.click(x, y)`.
    pub async fn click(&self, x: f64, y: f64) -> Result<()> {
        self.page.click_at(x, y, 1).await
    }

    /// `mouse.dblclick(x, y)`.
    pub async fn dblclick(&self, x: f64, y: f64) -> Result<()> {
        self.page.click_at(x, y, 2).await
    }

    /// `mouse.wheel(deltaX, deltaY)` at the mouse position.
    pub async fn wheel(&self, delta_x: f64, delta_y: f64) -> Result<()> {
        let e = self
            .page
            .inner
            .mouse
            .lock()
            .unwrap()
            .wheel(delta_x, delta_y);
        self.page.dispatch_mouse(e).await
    }
}

/// `page.keyboard()` on WebKit (`Input.dispatchKeyEvent`).
#[derive(Clone, Debug)]
pub struct Keyboard {
    page: Page,
}

impl Keyboard {
    /// `keyboard.down(key)`.
    pub async fn down(&self, key: &str) -> Result<()> {
        self.page.key(key, true).await
    }
    /// `keyboard.up(key)`.
    pub async fn up(&self, key: &str) -> Result<()> {
        self.page.key(key, false).await
    }
    /// `keyboard.press(shortcut)`: `"Enter"`, `"Control+a"`, ...
    pub async fn press(&self, shortcut: &str) -> Result<()> {
        self.page.press_shortcut(shortcut).await
    }
    /// `keyboard.type(text)`: key by key (characters off the keyboard are
    /// inserted).
    pub async fn type_text(&self, text: &str) -> Result<()> {
        use crate::domain::input::{TypeStep, type_steps};
        for step in type_steps(text) {
            match step {
                TypeStep::Press(key) => self.page.press_shortcut(&key).await?,
                TypeStep::Insert(text) => self.page.insert_text(&text).await?,
            }
        }
        Ok(())
    }
    /// `keyboard.insertText(text)`.
    pub async fn insert_text(&self, text: &str) -> Result<()> {
        self.page.insert_text(text).await
    }
}

impl Locator {
    /// `locator.filter({ hasText, hasNotText, has, hasNot, visible })`.
    pub fn filter(&self, filter: Filter) -> Locator {
        self.with(compose::filter(self.selector(), &filter))
    }
    /// `locator.and(other)`: elements matching both.
    pub fn and(&self, other: &Locator) -> Locator {
        self.with(compose::and(self.selector(), other.selector()))
    }
    /// `locator.or(other)`: elements matching either.
    pub fn or(&self, other: &Locator) -> Locator {
        self.with(compose::or(self.selector(), other.selector()))
    }

    /// `locator.all()`: one locator per currently matching element.
    pub async fn all(&self) -> Result<Vec<Locator>> {
        let n = self.count().await?;
        Ok((0..n as i64).map(|i| self.nth(i)).collect())
    }

    /// `locator.boundingBox()`: the element's box in the page's viewport
    /// (CSS px), `None` when it is not rendered.
    pub async fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        let offset = self.scope().await?.map(|(_, o)| o).unwrap_or_default();
        let v = self
            .read(
                "(i, node) => { const r = node.getBoundingClientRect(); \
                 if (!r.width && !r.height && !node.getClientRects().length) return null; \
                 return { x: r.x, y: r.y, width: r.width, height: r.height }; }",
                &[],
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        let n = |k: &str| v[k].as_f64().unwrap_or_default();
        Ok(Some(BoundingBox {
            x: n("x") + offset.0,
            y: n("y") + offset.1,
            width: n("width"),
            height: n("height"),
        }))
    }

    /// `locator.screenshot()`: a PNG of the element's box, scrolled into
    /// view first.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        self.read(
            "(i, node) => { node.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); }",
            &[],
        )
        .await?;
        let b = self.bounding_box().await?.ok_or_else(|| {
            Error::Evaluation(format!(
                "element is not visible: locator('{}')",
                self.selector()
            ))
        })?;
        self.page()
            .screenshot()
            .clip(Rect {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            })
            .await
    }

    /// `locator.evaluate(fn)`: `fn(element)` (awaited) with a JSON result.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_with(function, Value::Null).await
    }

    /// `locator.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self
            .read(
                &format!("(i, node, arg) => ({function})(node, arg)"),
                &[arg],
            )
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }
}

impl FrameLocator {
    /// `frameLocator.nth(index)`.
    pub fn nth(&self, index: i64) -> FrameLocator {
        let mut frames = self.frames.clone();
        if let Some(last) = frames.last_mut() {
            *last = compose::nth(last, index);
        }
        FrameLocator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames,
        }
    }
    pub fn first(&self) -> FrameLocator {
        self.nth(0)
    }
    pub fn last(&self) -> FrameLocator {
        self.nth(-1)
    }

    /// `frameLocator.owner()`: the `<iframe>` element itself.
    pub fn owner(&self) -> Locator {
        let mut frames = self.frames.clone();
        let selector = frames.pop().unwrap_or_default();
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames,
            selector,
        }
    }
}

impl ContextInner {
    /// Sends an event to `context.events()` subscribers; false if none.
    pub(crate) fn emit(&self, event: ContextEvent) -> bool {
        if let ContextEvent::Page(p) = &event {
            self.recorders.page_created(p);
        }
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|s| s.unbounded_send(event.clone()).is_ok());
        !subs.is_empty()
    }
}

impl BrowserContext {
    /// `context.browser()`.
    pub fn browser(&self) -> Option<Browser> {
        Some(self.inner.browser.clone())
    }

    /// `context.pages()`.
    pub fn pages(&self) -> Vec<Page> {
        self.live_pages()
            .into_iter()
            .filter(|p| !p.is_closed())
            .collect()
    }

    /// `context.isClosed()`.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// `context.backgroundPages()`: extension pages, Chromium only.
    pub fn background_pages(&self) -> Vec<Page> {
        Vec::new()
    }

    /// `context.serviceWorkers()`: Playwright reports service workers in
    /// Chromium only.
    pub fn service_workers(&self) -> Vec<Worker> {
        Vec::new()
    }

    /// `context.newCDPSession(page)`: Chromium only.
    pub async fn new_cdp_session(&self, _page: &Page) -> Result<()> {
        Err(chromium_only("context.newCDPSession()"))
    }

    /// `context.setDefaultTimeout(timeout)`: for every page's actions and
    /// waits (a page's own setting wins).
    pub fn set_default_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().action = Some(timeout);
    }

    /// `context.setDefaultNavigationTimeout(timeout)`.
    pub fn set_default_navigation_timeout(&self, timeout: Duration) {
        self.inner.timeouts.lock().unwrap().navigation = Some(timeout);
    }

    /// Ends a `page.pause()` (what Playwright's inspector "Resume" does).
    pub fn resume(&self) {
        self.inner.paused.send_replace(false);
    }

    /// `context.events()`: pages opened, their console messages, dialogs,
    /// requests and errors, and the context's close, from now on.
    pub fn events(&self) -> impl futures::Stream<Item = ContextEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// `context.waitForEvent(event)`: the first event `matcher` maps to a
    /// value. Listens from the call on, so start it before the action.
    pub fn wait_for_event<T, F>(
        &self,
        matcher: F,
    ) -> impl Future<Output = Result<T>> + Send + use<T, F>
    where
        F: Fn(&ContextEvent) -> Option<T> + Send + Sync + 'static,
        T: Send + 'static,
    {
        let mut events = self.events();
        let clock = self.inner.browser.inner.services.clock.clone();
        let timeout = self
            .inner
            .timeouts
            .lock()
            .unwrap()
            .action
            .unwrap_or(crate::application::firefox::DEFAULT_TIMEOUT);
        async move {
            with_timeout(&*clock, timeout, "waiting for context event", async {
                while let Some(e) = events.next().await {
                    if let Some(v) = matcher(&e) {
                        return Ok(v);
                    }
                    if matches!(e, ContextEvent::Close) {
                        break;
                    }
                }
                Err(Error::TargetClosed("context closed".into()))
            })
            .await
        }
    }

    /// `context.setExtraHTTPHeaders(headers)`: for every page (a page's own
    /// headers win).
    pub async fn set_extra_http_headers(&self, headers: &[(&str, &str)]) -> Result<()> {
        *self.inner.extra_headers.lock().unwrap() = headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        for page in self.pages() {
            page.apply_headers().await?;
        }
        Ok(())
    }

    /// `context.setOffline(offline)`.
    pub async fn set_offline(&self, offline: bool) -> Result<()> {
        self.inner.offline.store(offline, Ordering::SeqCst);
        for page in self.pages() {
            page.target_call(
                "Network.setEmulateOfflineState",
                json!({"offline": offline}),
            )
            .await?;
        }
        Ok(())
    }

    /// `context.setGeolocation(geolocation)`; `None` emulates an
    /// unavailable position.
    pub async fn set_geolocation(&self, geolocation: Option<Geolocation>) -> Result<()> {
        let mut params = json!({"browserContextId": self.inner.id});
        if let Some(g) = geolocation {
            params["geolocation"] = json!({"timestamp": 0, "latitude": g.latitude,
                "longitude": g.longitude, "accuracy": g.accuracy});
        }
        self.wk()
            .call(None, "Playwright.setGeolocationOverride", params)
            .await
            .map(drop)
    }

    /// `context.grantPermissions(permissions, { origin })` (every origin
    /// when `origin` is `None`).
    pub async fn grant_permissions(
        &self,
        permissions: &[&str],
        origin: Option<&str>,
    ) -> Result<()> {
        let origin = origin.unwrap_or("*").to_owned();
        let list: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
        {
            let mut granted = self.inner.granted.lock().unwrap();
            match granted.iter_mut().find(|(o, _)| *o == origin) {
                Some((_, existing)) => {
                    for p in &list {
                        if !existing.contains(p) {
                            existing.push(p.clone());
                        }
                    }
                }
                None => granted.push((origin.clone(), list)),
            }
        }
        let all = self
            .inner
            .granted
            .lock()
            .unwrap()
            .iter()
            .find(|(o, _)| *o == origin)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        for page in self.pages() {
            page.proxy_call(
                "Emulation.grantPermissions",
                json!({"origin": origin, "permissions": all}),
            )
            .await?;
        }
        Ok(())
    }

    /// `context.clearPermissions()`.
    pub async fn clear_permissions(&self) -> Result<()> {
        self.inner.granted.lock().unwrap().clear();
        for page in self.pages() {
            page.proxy_call("Emulation.resetPermissions", json!({}))
                .await?;
        }
        Ok(())
    }

    /// `context.setHTTPCredentials(credentials)` (deprecated in Playwright).
    pub async fn set_http_credentials(&self, credentials: Option<HttpCredentials>) -> Result<()> {
        *self.inner.credentials.lock().unwrap() = credentials.clone();
        for page in self.pages() {
            page.apply_credentials(credentials.as_ref()).await?;
        }
        Ok(())
    }

    /// `context.storageState()`: cookies, and the localStorage of the
    /// origins the context's pages show.
    pub async fn storage_state(&self) -> Result<StorageState> {
        let cookies = self.cookies().await?;
        let mut origins: Vec<OriginState> = Vec::new();
        for page in self.pages() {
            let v: Value = match page
                .evaluate(
                    "(() => ({ origin: location.origin, items: Object.keys(localStorage)\
                     .map(name => ({ name, value: localStorage.getItem(name) })) }))()",
                )
                .await
            {
                Ok(v) => v,
                Err(_) => continue,
            };
            let origin = v["origin"].as_str().unwrap_or_default().to_owned();
            if origin.is_empty() || origin == "null" {
                continue;
            }
            let items: Vec<NameValue> = v["items"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|kv| NameValue {
                            name: kv["name"].as_str().unwrap_or_default().to_owned(),
                            value: kv["value"].as_str().unwrap_or_default().to_owned(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if items.is_empty() || origins.iter().any(|o| o.origin == origin) {
                continue;
            }
            origins.push(OriginState {
                origin,
                local_storage: items,
                indexed_db: None,
                opfs: None,
            });
        }
        Ok(StorageState { cookies, origins })
    }

    /// `context.setStorageState(state)`: replaces cookies and localStorage.
    pub async fn set_storage_state(&self, state: &StorageState) -> Result<()> {
        self.clear_cookies().await?;
        for page in self.pages() {
            let _ = page
                .evaluate::<Value>("(() => { try { localStorage.clear(); } catch {} })()")
                .await;
        }
        if !state.cookies.is_empty() {
            let cookies: Vec<NewCookie> = state.cookies.iter().map(Cookie::to_new_cookie).collect();
            self.add_cookies(&cookies).await?;
        }
        for origin in state.origins.iter().filter(|o| !o.local_storage.is_empty()) {
            let page = self.new_page().await?;
            page.route("**/*", |route| async move {
                let _ = route
                    .fulfill()
                    .content_type("text/html")
                    .body("<html></html>")
                    .await;
            })
            .await?;
            let items: Vec<Value> = origin
                .local_storage
                .iter()
                .map(|kv| json!({"name": kv.name, "value": kv.value}))
                .collect();
            let written = async {
                page.goto(&format!("{}/", origin.origin.trim_end_matches('/')))
                    .await?;
                page.evaluate_with::<Value, _>(
                    "(items) => { for (const { name, value } of items) localStorage.setItem(name, value); }",
                    items,
                )
                .await
                .map(drop)
            }
            .await;
            page.close().await?;
            written?;
        }
        Ok(())
    }
}

/// `browser.browserType()` on WebKit.
#[derive(Debug, Clone, Copy)]
pub struct BrowserType;

impl BrowserType {
    /// `browserType.name()`.
    pub fn name(&self) -> &'static str {
        "webkit"
    }
}

impl Browser {
    /// `browser.browserType()`.
    pub fn browser_type(&self) -> BrowserType {
        BrowserType
    }

    /// `browser.isConnected()`.
    pub fn is_connected(&self) -> bool {
        !self.inner.closed.load(Ordering::SeqCst)
    }

    /// `browser.contexts()`: the open contexts made through this browser.
    pub fn contexts(&self) -> Vec<BrowserContext> {
        let mut list = self.inner.contexts.lock().unwrap();
        list.retain(|c| {
            c.upgrade()
                .is_some_and(|c| !c.closed.load(Ordering::SeqCst))
        });
        list.iter()
            .filter_map(|c| c.upgrade())
            .map(|inner| BrowserContext { inner })
            .collect()
    }

    /// `browser.newBrowserCDPSession()`: Chromium only.
    pub async fn new_browser_cdp_session(&self) -> Result<()> {
        Err(chromium_only("browser.newBrowserCDPSession()"))
    }

    /// `browser.startTracing()` (Chrome's performance trace): Chromium only.
    pub async fn start_tracing(&self, _page: Option<&Page>, _categories: &[&str]) -> Result<()> {
        Err(chromium_only("browser.startTracing()"))
    }

    /// `browser.stopTracing()`: Chromium only.
    pub async fn stop_tracing(&self) -> Result<Vec<u8>> {
        Err(chromium_only("browser.stopTracing()"))
    }
}

/// A WebKit context as an API request context's cookie jar.
struct WebKitJar(std::sync::Weak<ContextInner>);

impl WebKitJar {
    fn context(&self) -> Result<BrowserContext> {
        self.0
            .upgrade()
            .filter(|c| !c.closed.load(Ordering::SeqCst))
            .map(|inner| BrowserContext { inner })
            .ok_or_else(|| Error::Request("Request context disposed.".into()))
    }
}

impl crate::application::request::CookieStore for WebKitJar {
    fn is_open(&self) -> bool {
        self.context().is_ok()
    }
    fn cookies_for<'a>(
        &'a self,
        urls: &'a [&'a str],
    ) -> futures::future::BoxFuture<'a, Result<Vec<Cookie>>> {
        Box::pin(async move { self.context()?.cookies_for(urls).await })
    }
    fn add_cookies<'a>(
        &'a self,
        cookies: &'a [NewCookie],
    ) -> futures::future::BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.context()?.add_cookies(cookies).await })
    }
    fn clear_cookies_matching(
        &self,
        filter: crate::domain::storage::ClearCookies,
    ) -> futures::future::BoxFuture<'_, Result<()>> {
        Box::pin(async move { self.context()?.clear_cookies_matching(filter).await })
    }
    fn storage_state(&self) -> futures::future::BoxFuture<'_, Result<StorageState>> {
        Box::pin(async move { self.context()?.storage_state().await })
    }
    fn user_agent(&self) -> futures::future::BoxFuture<'_, Option<String>> {
        Box::pin(async move {
            let ctx = self.context().ok()?;
            for page in ctx.pages() {
                if let Ok(ua) = page.evaluate::<String>("navigator.userAgent").await {
                    return Some(ua);
                }
            }
            Some(crate::domain::webkit::default_user_agent())
        })
    }
}

impl BrowserContext {
    /// `context.request`: API requests with this context's cookies,
    /// headers, credentials and base URL (Playwright's APIRequestContext).
    pub fn request(&self) -> crate::application::request::ApiRequestContext {
        let o = &self.inner.options;
        crate::application::request::ApiRequestContext::with_store(
            self.inner.browser.inner.services.clone(),
            crate::application::request::ApiRequestOptions {
                base_url: o.base_url.clone(),
                extra_http_headers: self
                    .inner
                    .extra_headers
                    .lock()
                    .unwrap()
                    .iter()
                    .cloned()
                    .collect(),
                http_credentials: self.inner.credentials.lock().unwrap().clone(),
                user_agent: o.user_agent.clone(),
                ignore_https_errors: o.ignore_https_errors,
                timeout: None,
                fail_on_status_code: false,
                storage_state: None,
                client_certificates: o.client_certificates.clone(),
            },
            std::sync::Arc::new(WebKitJar(std::sync::Arc::downgrade(&self.inner))),
        )
    }

    /// `context.clearCookies({ name, domain, path })`: only matching
    /// cookies (the others are put back).
    pub async fn clear_cookies_matching(
        &self,
        filter: crate::domain::storage::ClearCookies,
    ) -> Result<()> {
        let keep: Vec<NewCookie> = self
            .cookies()
            .await?
            .iter()
            .filter(|c| !filter.matches(c))
            .map(Cookie::to_new_cookie)
            .collect();
        self.clear_cookies().await?;
        if !keep.is_empty() {
            self.add_cookies(&keep).await?;
        }
        Ok(())
    }
}

impl Page {
    /// `page.request`: the context's API request context.
    pub fn request(&self) -> crate::application::request::ApiRequestContext {
        self.context()
            .expect("a page belongs to a context while it is open")
            .request()
    }
}

impl Page {
    /// `Network.webSocket*` events (`WKPage`'s WebSocket reporting).
    pub(crate) fn on_websocket_event(&self, method: &str, p: &Value) {
        use crate::application::websocket::{WebSocket, WebSocketEvent, payload};
        let id = p["requestId"].as_str().unwrap_or_default().to_owned();
        if method == "Network.webSocketCreated" {
            let ws = WebSocket::new(p["url"].as_str().unwrap_or_default().to_owned());
            self.inner.websockets.lock().unwrap().insert(id, ws.clone());
            self.emit(PageEvent::WebSocket(ws));
            return;
        }
        let ws = {
            let mut map = self.inner.websockets.lock().unwrap();
            if method == "Network.webSocketClosed" {
                map.remove(&id)
            } else {
                map.get(&id).cloned()
            }
        };
        let Some(ws) = ws else {
            return;
        };
        // WebKit also reports control frames (close, ping, pong).
        if p.pointer("/response/opcode").and_then(Value::as_u64) >= Some(8) {
            return;
        }
        match method {
            "Network.webSocketFrameSent" => ws.push(WebSocketEvent::FrameSent(payload(p))),
            "Network.webSocketFrameReceived" => ws.push(WebSocketEvent::FrameReceived(payload(p))),
            "Network.webSocketFrameError" => ws.push(WebSocketEvent::Error(
                p["errorMessage"].as_str().unwrap_or_default().to_owned(),
            )),
            "Network.webSocketClosed" => ws.push(WebSocketEvent::Close),
            _ => {}
        }
    }

    /// `Page.fileChooserOpened`: a `FileChooser` for the input.
    pub(crate) fn on_file_chooser(&self, p: &Value) {
        let frame = p["frameId"]
            .as_str()
            .filter(|f| *f != self.main_frame_id())
            .map(str::to_owned);
        let handle = crate::application::webkit_handle::JsHandle {
            page: self.clone(),
            frame,
            remote: p["element"].clone(),
            worker: None,
        };
        let Some(element) = handle.as_element() else {
            return;
        };
        let page = self.clone();
        tokio::spawn(async move {
            let multiple = element
                .evaluate::<bool>("e => !!e.multiple")
                .await
                .unwrap_or(false);
            page.emit(PageEvent::FileChooser(
                crate::application::webkit_events::FileChooser {
                    page: page.clone(),
                    element,
                    multiple,
                },
            ));
        });
    }

    /// The page went away: its sockets close, its workers end.
    pub(crate) fn on_page_gone(&self) {
        use crate::application::websocket::WebSocketEvent;
        let sockets: Vec<_> = self
            .inner
            .websockets
            .lock()
            .unwrap()
            .drain()
            .map(|(_, ws)| ws)
            .collect();
        for ws in sockets {
            ws.push(WebSocketEvent::Close);
        }
        for (_, w) in self.inner.workers.lock().unwrap().drain() {
            w.mark_closed();
        }
    }

    /// `page.touchscreen()`.
    pub fn touchscreen(&self) -> Touchscreen {
        Touchscreen { page: self.clone() }
    }
}

/// `page.touchscreen()` on WebKit (`Input.dispatchTapEvent`).
#[derive(Clone, Debug)]
pub struct Touchscreen {
    page: Page,
}

impl Touchscreen {
    /// `touchscreen.tap(x, y)`: only in a context with `hasTouch`, like
    /// Playwright.
    pub async fn tap(&self, x: f64, y: f64) -> Result<()> {
        if !self.page.inner.options.has_touch {
            return Err(Error::InvalidArgument(
                "The page does not support tap. Use hasTouch context option to enable touch support."
                    .into(),
            ));
        }
        self.page.tap_at(x, y).await
    }
}

type HandlerFn =
    std::sync::Arc<dyn Fn(Locator) -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

/// One `page.addLocatorHandler` registration.
pub(crate) struct LocatorHandler {
    locator: Locator,
    handler: HandlerFn,
    times_left: Option<usize>,
    no_wait_after: bool,
}

impl Page {
    /// `page.addLocatorHandler(locator, handler)`: before an action, when
    /// `locator` is visible, `handler` runs (e.g. to dismiss an overlay),
    /// then the action waits for it to be hidden.
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
        use futures::FutureExt;
        self.inner
            .locator_handlers
            .lock()
            .unwrap()
            .push(LocatorHandler {
                locator: locator.clone(),
                handler: std::sync::Arc::new(move |l| handler(l).boxed()),
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
    pub(crate) async fn run_locator_handlers(&self) {
        if self.inner.locator_handlers.lock().unwrap().is_empty()
            || self.inner.handling_locator.swap(true, Ordering::SeqCst)
        {
            return;
        }
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
                let deadline = self.now() + self.timeout();
                while locator.is_visible().await.unwrap_or(false) && self.now() < deadline {
                    self.sleep(Duration::from_millis(50)).await;
                }
            }
        }
        self.inner.handling_locator.store(false, Ordering::SeqCst);
    }
}

impl Page {
    /// `page.pdf()`: Playwright prints PDFs in Chromium only.
    pub async fn pdf(&self) -> Result<Vec<u8>> {
        Err(chromium_only("page.pdf()"))
    }
}
