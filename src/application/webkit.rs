//! The WebKit engine: the Playwright API over Playwright's WebKit protocol
//! (`chromiumoxide::webkit`), driving its WebKit build through
//! `--inspector-pipe`. See `docs/ARCHITECTURE.md`, "Engines".
//!
//! Like Playwright's `WKPage`: a page is a *page proxy* (`pageProxyId`)
//! whose document lives in a *page target*; a cross-process navigation
//! creates a provisional target that replaces the current one when it
//! commits (`Target.didCommitProvisionalTarget`).

use std::collections::HashMap;
use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::actions::{ADD_STYLE, StyleTag};
use crate::application::screenshot::{CLEANUP, FULL_PAGE_SIZE, PREPARE};
use crate::application::services::Services;
use crate::application::webkit_events::{ConsoleMessage, PageEvent};
use crate::domain::emulation::{ContextOptions, Viewport};
use crate::domain::error::{Error, Result};
use crate::domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale, Size, Target,
    plan_page, validate,
};
use crate::domain::webkit::{BROWSER_VERSION, command_line, default_user_agent};
use crate::ports::{BrowserProcess, EventStream, WebKitCommand, WebKitSession};

/// Playwright's utility world name.
const UTILITY: &str = "__playwright_utility_world__";

/// How long to wait for page targets, loads and `wait_for_function`.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Options for [`crate::WebKit::launch`] (Playwright's `webkit.launch`).
#[derive(Debug, Clone)]
pub struct WebKitLaunchOptions {
    /// Playwright's WebKit (`webkit-<revision>/pw_run.sh`, or
    /// `Playwright.exe` on Windows); found in Playwright's folders when
    /// `None`.
    pub executable_path: Option<PathBuf>,
    /// Headless (default `true`).
    pub headless: bool,
    /// Extra command-line arguments.
    pub args: Vec<String>,
    /// Extra environment variables for the browser process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for WebKit to start (default 180 s).
    pub timeout: Duration,
}

impl Default for WebKitLaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            headless: true,
            args: Vec::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(180),
        }
    }
}

/// The newest `webkit-<revision>` Playwright installed.
fn find_webkit(services: &Services) -> Option<PathBuf> {
    let dirs = crate::domain::firefox::playwright_dirs(
        services
            .env
            .var("PLAYWRIGHT_BROWSERS_PATH")
            .map(PathBuf::from),
        services.env.home_dir(),
        services.env.var("LOCALAPPDATA").map(PathBuf::from),
        cfg!(windows),
    );
    let exe = if cfg!(windows) {
        "Playwright.exe"
    } else {
        "pw_run.sh"
    };
    let mut found: Vec<(u32, PathBuf)> = dirs
        .iter()
        .flat_map(|d| {
            services
                .fs
                .list_dir(d)
                .into_iter()
                .filter_map(|name| {
                    let rev = name.strip_prefix("webkit-")?.parse().ok()?;
                    let path = d.join(&name).join(exe);
                    services.fs.is_file(&path).then_some((rev, path))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// Waits for `f` at most `timeout` (on the clock port).
async fn within<T>(
    services: &Services,
    timeout: Duration,
    what: &str,
    f: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    let sleep = services.clock.sleep(timeout);
    futures::pin_mut!(f);
    match futures::future::select(f, sleep).await {
        futures::future::Either::Left((r, _)) => r,
        futures::future::Either::Right(_) => Err(Error::Timeout {
            ms: timeout.as_millis() as u64,
            action: what.to_owned(),
        }),
    }
}

/// A WebKit browser (Playwright's WebKit build).
#[derive(Clone)]
pub struct Browser {
    inner: Arc<BrowserInner>,
}

struct BrowserInner {
    wk: Arc<dyn WebKitSession>,
    services: Services,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Browser").finish()
    }
}

impl Browser {
    /// Starts WebKit with Playwright's arguments and enables its protocol.
    pub async fn launch(services: Services, options: WebKitLaunchOptions) -> Result<Browser> {
        let program = match options.executable_path.clone() {
            Some(p) => p,
            None => find_webkit(&services).ok_or_else(|| {
                Error::Launch(
                    "no WebKit found: install Playwright's (`npx playwright install webkit`) \
                     or set executable_path"
                        .into(),
                )
            })?,
        };
        let launched = services
            .launcher
            .launch_webkit(WebKitCommand {
                program,
                args: command_line(options.headless, None, &options.args, cfg!(windows)),
                env: options.env.clone(),
                timeout_ms: options.timeout.as_millis() as u64,
            })
            .await?;
        let enabled = within(
            &services,
            options.timeout,
            "waiting for WebKit to start",
            launched.session.call(None, "Playwright.enable", json!({})),
        )
        .await;
        if let Err(e) = enabled {
            launched.process.shutdown().await;
            return Err(e);
        }
        Ok(Browser {
            inner: Arc::new(BrowserInner {
                wk: launched.session,
                services,
                process: Mutex::new(Some(launched.process)),
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// `browser.version()`: the Safari version Playwright reports.
    pub async fn version(&self) -> Result<String> {
        Ok(BROWSER_VERSION.to_owned())
    }

    /// `browser.newContext(options)`.
    pub async fn new_context(&self, options: ContextOptions) -> Result<BrowserContext> {
        let wk = &self.inner.wk;
        let id = wk
            .call(None, "Playwright.createContext", json!({}))
            .await?
            .get("browserContextId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let context = BrowserContext {
            inner: Arc::new(ContextInner {
                browser: self.clone(),
                id: id.clone(),
                options: Arc::new(options),
                init_scripts: Mutex::new(Vec::new()),
                pages: Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
            }),
        };
        if let Err(e) = context.initialize().await {
            let _ = context.close().await;
            return Err(e);
        }
        Ok(context)
    }

    /// `browser.newPage()`: a page in a new context, closed with it.
    pub async fn new_page(&self) -> Result<Page> {
        let context = self.new_context(ContextOptions::default()).await?;
        let page = context.new_page().await?;
        *page.inner.owned_context.lock().unwrap() = Some(context);
        Ok(page)
    }

    /// `browser.close()`.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = within(
            &self.inner.services,
            Duration::from_secs(5),
            "closing WebKit",
            self.inner.wk.call(None, "Playwright.close", json!({})),
        )
        .await;
        let process = self.inner.process.lock().unwrap().take();
        if let Some(process) = process {
            process.shutdown().await;
        }
        Ok(())
    }
}

/// A WebKit browser context.
#[derive(Clone)]
pub struct BrowserContext {
    pub(crate) inner: Arc<ContextInner>,
}

pub(crate) struct ContextInner {
    browser: Browser,
    pub(crate) id: String,
    options: Arc<ContextOptions>,
    /// `context.addInitScript` sources.
    pub(crate) init_scripts: Mutex<Vec<String>>,
    /// The context's pages (to update their bootstrap script).
    pub(crate) pages: Mutex<Vec<std::sync::Weak<PageInner>>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::BrowserContext")
            .field("id", &self.inner.id)
            .finish()
    }
}

impl BrowserContext {
    pub(crate) fn wk(&self) -> &Arc<dyn WebKitSession> {
        &self.inner.browser.inner.wk
    }

    /// The context-wide settings (`WKBrowserContext.initialize`).
    async fn initialize(&self) -> Result<()> {
        let o = &self.inner.options;
        let id = &self.inner.id;
        let wk = self.wk();
        wk.call(
            None,
            "Playwright.setDownloadBehavior",
            json!({"behavior": "deny", "browserContextId": id}),
        )
        .await?;
        if o.ignore_https_errors {
            wk.call(
                None,
                "Playwright.setIgnoreCertificateErrors",
                json!({"browserContextId": id, "ignore": true}),
            )
            .await?;
        }
        if let Some(locale) = &o.locale {
            wk.call(
                None,
                "Playwright.setLanguages",
                json!({"browserContextId": id, "languages": [locale]}),
            )
            .await?;
        }
        if let Some(g) = o.geolocation {
            wk.call(
                None,
                "Playwright.setGeolocationOverride",
                json!({"browserContextId": id, "geolocation": {"timestamp": 0,
                       "latitude": g.latitude, "longitude": g.longitude, "accuracy": g.accuracy}}),
            )
            .await?;
        }
        Ok(())
    }

    /// `context.newPage()`.
    pub async fn new_page(&self) -> Result<Page> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("context has been closed".into()));
        }
        let services = self.inner.browser.inner.services.clone();
        // Subscribed before the page exists, so no target event is missed.
        let events = self.wk().subscribe();
        let created = self
            .wk()
            .call(
                None,
                "Playwright.createPage",
                json!({"browserContextId": self.inner.id}),
            )
            .await?;
        let proxy = created
            .get("pageProxyId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let (ready, _) = watch::channel(false);
        let (loads, _) = watch::channel(0u64);
        let inner = Arc::new(PageInner {
            wk: self.wk().clone(),
            services: services.clone(),
            proxy,
            options: self.inner.options.clone(),
            target: Mutex::new(String::new()),
            frame_id: Mutex::new(String::new()),
            url: Mutex::new("about:blank".to_owned()),
            worlds: Mutex::new(HashMap::new()),
            viewport: Mutex::new(None),
            ready,
            loads,
            load_error: Mutex::new(None),
            owned_context: Mutex::new(None),
            keyboard: tokio::sync::Mutex::new(crate::domain::input::Keyboard::new(cfg!(
                target_os = "macos"
            ))),
            mouse: Mutex::new(crate::domain::input::Mouse::default()),
            context: Arc::downgrade(&self.inner),
            init_scripts: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            last_console: Mutex::new(None),
            routes: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        });
        self.inner
            .pages
            .lock()
            .unwrap()
            .push(Arc::downgrade(&inner));
        let page = Page { inner };
        page.follow(events);
        let mut ready = page.inner.ready.subscribe();
        within(&services, TIMEOUT, "waiting for the page target", async {
            ready
                .wait_for(|r| *r)
                .await
                .map(drop)
                .map_err(|_| Error::TargetClosed("page closed while opening".into()))
        })
        .await?;
        Ok(page)
    }

    /// `context.close()`.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.wk()
            .call(
                None,
                "Playwright.deleteContext",
                json!({"browserContextId": self.inner.id}),
            )
            .await?;
        Ok(())
    }
}

/// A target's execution context ids: (main world, utility world).
type Worlds = (Option<i64>, Option<i64>);

/// A WebKit page.
#[derive(Clone)]
pub struct Page {
    pub(crate) inner: Arc<PageInner>,
}

pub(crate) struct PageInner {
    pub(crate) wk: Arc<dyn WebKitSession>,
    services: Services,
    /// The page proxy id.
    pub(crate) proxy: String,
    pub(crate) options: Arc<ContextOptions>,
    /// The current page target.
    target: Mutex<String>,
    /// The main frame id.
    frame_id: Mutex<String>,
    url: Mutex<String>,
    /// Execution context ids per target: (main world, utility world).
    worlds: Mutex<HashMap<String, Worlds>>,
    /// `set_viewport_size` (else the context's viewport).
    viewport: Mutex<Option<Viewport>>,
    /// The first page target is set up.
    ready: watch::Sender<bool>,
    /// Main-frame loads so far (`Page.loadEventFired`).
    pub(crate) loads: watch::Sender<u64>,
    /// The last failed navigation (`Playwright.provisionalLoadFailed`).
    pub(crate) load_error: Mutex<Option<String>>,
    owned_context: Mutex<Option<BrowserContext>>,
    /// The keyboard and mouse models (`page.keyboard`, `page.mouse`).
    keyboard: tokio::sync::Mutex<crate::domain::input::Keyboard>,
    mouse: Mutex<crate::domain::input::Mouse>,
    /// The context (for its init scripts).
    pub(crate) context: std::sync::Weak<ContextInner>,
    /// `page.addInitScript` sources.
    pub(crate) init_scripts: Mutex<Vec<String>>,
    /// `page.events()` listeners.
    pub(crate) subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<PageEvent>>>,
    /// The last console message, for `Console.messageRepeatCountUpdated`.
    pub(crate) last_console: Mutex<Option<(ConsoleMessage, u64)>>,
    /// `page.route` handlers, oldest first.
    pub(crate) routes: Mutex<Vec<crate::application::webkit_route::RouteEntry>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Page")
            .field("proxy", &self.inner.proxy)
            .finish()
    }
}

impl Page {
    pub(crate) fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("page has been closed".into()));
        }
        Ok(())
    }

    pub(crate) fn target(&self) -> String {
        self.inner.target.lock().unwrap().clone()
    }

    /// A command to the page proxy.
    pub(crate) async fn proxy_call(&self, method: &str, params: Value) -> Result<Value> {
        self.inner
            .wk
            .call(Some(&self.inner.proxy), method, params)
            .await
    }

    /// A command to the current page target.
    pub(crate) async fn target_call(&self, method: &str, params: Value) -> Result<Value> {
        self.check_open()?;
        let target = self.target();
        self.inner
            .wk
            .call_target(&self.inner.proxy, &target, method, params)
            .await
    }

    /// The viewport Playwright gives a page: the page's, else the
    /// context's, else 1280x720.
    fn effective_viewport(&self) -> Viewport {
        self.inner
            .viewport
            .lock()
            .unwrap()
            .or(self.inner.options.viewport)
            .unwrap_or(Viewport::new(1280, 720))
    }

    /// Follows the page proxy's events: targets (provisional ones too),
    /// loads, navigations and execution contexts.
    fn follow(&self, mut events: EventStream) {
        let weak = Arc::downgrade(&self.inner);
        let proxy = self.inner.proxy.clone();
        let prefix = format!("{proxy}/");
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let Some(inner) = weak.upgrade() else { break };
                let page = Page { inner };
                let p = &event.params;
                match event.session_id.as_deref() {
                    None => {
                        if p["pageProxyId"].as_str() != Some(proxy.as_str()) {
                            continue;
                        }
                        match event.method.as_str() {
                            "Playwright.provisionalLoadFailed" => {
                                *page.inner.load_error.lock().unwrap() =
                                    Some(p["error"].as_str().unwrap_or("load failed").to_owned());
                                page.inner.loads.send_modify(|n| *n += 1);
                            }
                            "Playwright.pageProxyDestroyed" => {
                                page.inner.closed.store(true, Ordering::SeqCst);
                                page.emit(PageEvent::Close);
                                break;
                            }
                            _ => {}
                        }
                    }
                    Some(s) if s == proxy => match event.method.as_str() {
                        "Target.targetCreated" => {
                            let info = p["targetInfo"].clone();
                            let page = page.clone();
                            tokio::spawn(async move { page.setup_target(&info).await });
                        }
                        "Target.didCommitProvisionalTarget" => {
                            if let Some(new) = p["newTargetId"].as_str() {
                                *page.inner.target.lock().unwrap() = new.to_owned();
                            }
                        }
                        "Target.targetDestroyed" => {
                            if let Some(t) = p["targetId"].as_str() {
                                page.inner.worlds.lock().unwrap().remove(t);
                            }
                        }
                        "Dialog.javascriptDialogOpening" => page.on_dialog(p),
                        _ => {}
                    },
                    Some(s) if s.starts_with(&prefix) => {
                        let target = &s[prefix.len()..];
                        page.on_target_event(target, &event.method, p);
                    }
                    _ => {}
                }
            }
        });
    }

    fn on_target_event(&self, target: &str, method: &str, p: &Value) {
        let current = self.target() == target;
        match method {
            "Page.loadEventFired" if current => {
                self.inner.loads.send_modify(|n| *n += 1);
                self.emit(PageEvent::Load);
            }
            "Page.domContentEventFired" if current => {
                self.emit(PageEvent::DomContentLoaded);
            }
            "Console.messageAdded" if current => self.on_console(&p["message"]),
            "Network.requestIntercepted" => self.on_request_intercepted(target, p),
            "Console.messageRepeatCountUpdated" if current => {
                let count = p["count"].as_u64().unwrap_or(1);
                let repeated = {
                    let mut last = self.inner.last_console.lock().unwrap();
                    match last.as_mut() {
                        Some((m, seen)) if count > *seen => {
                            let n = count - *seen;
                            *seen = count;
                            Some((m.clone(), n))
                        }
                        _ => None,
                    }
                };
                if let Some((m, n)) = repeated {
                    for _ in 0..n {
                        self.emit(PageEvent::Console(m.clone()));
                    }
                }
            }
            "Page.frameNavigated" if p["frame"]["parentId"].is_null() => {
                if let Some(url) = p["frame"]["url"].as_str() {
                    *self.inner.url.lock().unwrap() = url.to_owned();
                }
                if let Some(id) = p["frame"]["id"].as_str() {
                    *self.inner.frame_id.lock().unwrap() = id.to_owned();
                }
            }
            "Page.navigatedWithinDocument" if current => {
                if let Some(url) = p["url"].as_str() {
                    *self.inner.url.lock().unwrap() = url.to_owned();
                }
            }
            "Runtime.executionContextCreated" => {
                let c = &p["context"];
                let main_frame = c["frameId"].as_str()
                    == Some(self.inner.frame_id.lock().unwrap().as_str())
                    || self.inner.frame_id.lock().unwrap().is_empty();
                if !main_frame {
                    return;
                }
                let id = c["id"].as_i64();
                let mut worlds = self.inner.worlds.lock().unwrap();
                let entry = worlds.entry(target.to_owned()).or_default();
                match (c["type"].as_str(), c["name"].as_str()) {
                    (Some("normal"), _) => entry.0 = id,
                    (_, Some(UTILITY)) => entry.1 = id,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Sets up a page target (`WKPage._initializeSession`), then resumes it.
    async fn setup_target(&self, info: &Value) {
        let target = info["targetId"].as_str().unwrap_or_default().to_owned();
        let provisional = info["isProvisional"].as_bool().unwrap_or(false);
        if info["type"].as_str() != Some("page") {
            return;
        }
        let first = !provisional && !*self.inner.ready.borrow();
        if first {
            *self.inner.target.lock().unwrap() = target.clone();
        }
        let wk = self.inner.wk.clone();
        let proxy = self.inner.proxy.clone();
        let call =
            |method: &'static str, params: Value| wk.call_target(&proxy, &target, method, params);
        let o = self.inner.options.clone();
        let mut steps = vec![
            call("Page.enable", json!({})),
            call("Runtime.enable", json!({})),
            call("Page.createUserWorld", json!({"name": UTILITY})),
            call(
                "Page.overrideUserAgent",
                json!({"value": o.user_agent.clone().unwrap_or_else(default_user_agent)}),
            ),
        ];
        steps.push(call(
            "Page.setBootstrapScript",
            json!({"source": self.bootstrap_script()}),
        ));
        steps.push(call("Network.enable", json!({})));
        steps.push(call("Console.enable", json!({})));
        steps.push(call(
            "Network.setExtraHTTPHeaders",
            json!({"headers": o.extra_http_headers}),
        ));
        steps.push(call(
            "Page.setTouchEmulationEnabled",
            json!({"enabled": o.has_touch}),
        ));
        for (setting, mobile_value) in [
            ("FullScreenEnabled", false),
            ("NotificationsEnabled", false),
            ("PointerLockEnabled", false),
            ("InputTypeMonthEnabled", true),
            ("InputTypeWeekEnabled", true),
            ("FixedBackgroundsPaintRelativeToDocument", true),
            ("PushAPIEnabled", false),
        ] {
            let value = if o.is_mobile {
                mobile_value
            } else {
                !mobile_value
            };
            steps.push(call(
                "Page.overrideSetting",
                json!({"setting": setting, "value": value}),
            ));
        }
        if let Some(tz) = &o.timezone_id {
            steps.push(call("Page.setTimeZone", json!({"timeZone": tz})));
        }
        let screen = o.screen.unwrap_or(self.effective_viewport());
        steps.push(call(
            "Page.setScreenSizeOverride",
            json!({"width": screen.width, "height": screen.height}),
        ));
        if let Ok(tree) = call("Page.getResourceTree", json!({})).await
            && !provisional
        {
            let frame = &tree["frameTree"]["frame"];
            if let Some(id) = frame["id"].as_str() {
                *self.inner.frame_id.lock().unwrap() = id.to_owned();
            }
            if let Some(url) = frame["url"].as_str() {
                *self.inner.url.lock().unwrap() = url.to_owned();
            }
        }
        for step in steps {
            let _ = step.await;
        }
        if self.has_routes() {
            let _ = crate::application::webkit_route::interception(self, &target, true).await;
        }
        if first {
            let v = self.effective_viewport();
            let _ = self.proxy_call("Dialog.enable", json!({})).await;
            let _ = self
                .proxy_call("Emulation.setActiveAndFocused", json!({"active": true}))
                .await;
            let _ = self
                .proxy_call(
                    "Emulation.setDeviceMetricsOverride",
                    json!({"width": v.width, "height": v.height, "fixedLayout": false,
                           "deviceScaleFactor": o.device_scale_factor.unwrap_or(1.0)}),
                )
                .await;
        }
        if info["isPaused"].as_bool() == Some(true) {
            let _ = self
                .proxy_call("Target.resume", json!({"targetId": target}))
                .await;
        }
        if first {
            self.inner.ready.send_replace(true);
        }
    }

    /// `page.goto(url)`: navigates and waits for the `load` event.
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.check_open()?;
        let mut loads = self.inner.loads.subscribe();
        let before = *loads.borrow_and_update();
        *self.inner.load_error.lock().unwrap() = None;
        let frame_id = self.inner.frame_id.lock().unwrap().clone();
        let mut params = json!({"url": url, "pageProxyId": self.inner.proxy});
        if !frame_id.is_empty() {
            params["frameId"] = json!(frame_id);
        }
        self.inner
            .wk
            .call(None, "Playwright.navigate", params)
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        self.wait_for_load(loads, before).await
    }

    /// Waits for a main-frame load after `before` loads; a failed one is a
    /// navigation error.
    pub(crate) async fn wait_for_load(
        &self,
        mut loads: watch::Receiver<u64>,
        before: u64,
    ) -> Result<()> {
        within(
            &self.inner.services,
            TIMEOUT,
            "waiting for the load event",
            async {
                loads
                    .wait_for(|n| *n > before)
                    .await
                    .map(drop)
                    .map_err(|_| Error::TargetClosed("page closed".into()))
            },
        )
        .await?;
        if let Some(error) = self.inner.load_error.lock().unwrap().take() {
            return Err(Error::Navigation(error));
        }
        Ok(())
    }

    /// `page.setContent(html)`: replaces the document and waits for it to
    /// finish loading.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.call_json(
            "(html) => { document.open(); document.write(html); document.close(); }",
            &[json!(html)],
        )
        .await?;
        self.wait_for_function("document.readyState === 'complete'")
            .await
    }

    /// `page.title()`.
    pub async fn title(&self) -> Result<String> {
        self.evaluate("document.title").await
    }

    /// `page.url()`: the main frame's URL.
    pub fn url(&self) -> String {
        self.inner.url.lock().unwrap().clone()
    }

    /// `page.content()`.
    pub async fn content(&self) -> Result<String> {
        self.evaluate(
            "(() => { let r = ''; if (document.doctype) r = new XMLSerializer().serializeToString(document.doctype); \
             if (document.documentElement) r += document.documentElement.outerHTML; return r; })()",
        )
        .await
    }

    /// `page.evaluate(expression or function)` with a JSON result.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.evaluate_value(source, None).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `page.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self.evaluate_value(source, Some(&arg)).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// Runs `expression` in the main world (or the utility world) and
    /// awaits it when it is a promise; the remote object of the result.
    async fn run(&self, expression: &str, utility: bool) -> Result<Value> {
        let target = self.target();
        let context = {
            let worlds = self.inner.worlds.lock().unwrap();
            let w = worlds.get(&target).copied().unwrap_or_default();
            if utility { w.1 } else { w.0 }
        };
        let mut params = json!({"expression": expression, "returnByValue": false,
                                "emulateUserGesture": true});
        if let Some(id) = context {
            params["contextId"] = json!(id);
        }
        let mut r = self.target_call("Runtime.evaluate", params).await?;
        if r["wasThrown"].as_bool() != Some(true)
            && r["result"]["className"].as_str() == Some("Promise")
            && let Some(id) = r["result"]["objectId"].as_str()
        {
            r = self
                .target_call(
                    "Runtime.awaitPromise",
                    json!({"promiseObjectId": id, "returnByValue": true}),
                )
                .await?;
        }
        if r["wasThrown"].as_bool() == Some(true) {
            return Err(Error::Evaluation(
                r["result"]["description"]
                    .as_str()
                    .unwrap_or("exception")
                    .to_owned(),
            ));
        }
        Ok(r["result"].take())
    }

    /// Playwright's string semantics: an expression is returned, a function
    /// is called with the argument; values come back as JSON. Sources that
    /// are statements rather than one expression run as a script.
    async fn evaluate_value(&self, source: &str, arg: Option<&Value>) -> Result<Value> {
        let source = source.trim().trim_end_matches(';');
        let arg = arg
            .map(Value::to_string)
            .unwrap_or_else(|| "undefined".into());
        let wrapped = format!(
            "(async (__chromiumoxide_arg) => {{ const __v = (\n{source}\n); \
             const __r = typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; \
             return __r === undefined ? undefined : JSON.stringify(__r); }})({arg})"
        );
        match self.run(&wrapped, false).await {
            Ok(v) => Ok(match v.get("value").and_then(Value::as_str) {
                Some(json) => serde_json::from_str(json)
                    .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}")))?,
                None => Value::Null,
            }),
            Err(Error::Evaluation(m)) if m.starts_with("SyntaxError") => {
                let r = self
                    .target_call(
                        "Runtime.evaluate",
                        json!({"expression": source, "returnByValue": true}),
                    )
                    .await?;
                if r["wasThrown"].as_bool() == Some(true) {
                    return Err(Error::Evaluation(
                        r["result"]["description"].as_str().unwrap_or(&m).to_owned(),
                    ));
                }
                Ok(r["result"].get("value").cloned().unwrap_or(Value::Null))
            }
            Err(e) => Err(e),
        }
    }

    /// Calls a function source with JSON arguments (main world); its JSON
    /// result.
    async fn call_json(&self, function: &str, args: &[Value]) -> Result<Value> {
        self.call_json_in(function, args, false).await
    }

    async fn call_json_in(&self, function: &str, args: &[Value], utility: bool) -> Result<Value> {
        let args: Vec<String> = args.iter().map(Value::to_string).collect();
        let expression = format!(
            "(async () => {{ const __r = await ({function})({}); \
             return __r === undefined ? undefined : JSON.stringify(__r); }})()",
            args.join(", ")
        );
        let v = self.run(&expression, utility).await?;
        match v.get("value").and_then(Value::as_str) {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    /// `page.addStyleTag({ content | url | path })`.
    pub async fn add_style_tag(&self, tag: StyleTag) -> Result<()> {
        let (content, url) = match tag {
            StyleTag::Content(css) => (Value::String(css), Value::Null),
            StyleTag::Url(u) => (Value::Null, Value::String(u)),
            StyleTag::Path(p) => {
                let bytes = self.inner.services.fs.read(&p)?;
                let mut css = String::from_utf8_lossy(&bytes).into_owned();
                css.push_str(&format!(
                    "\n/*# sourceURL={}*/",
                    p.display().to_string().replace('\n', "")
                ));
                (Value::String(css), Value::Null)
            }
        };
        self.call_json(ADD_STYLE, &[content, url]).await?;
        Ok(())
    }

    /// `page.setViewportSize(size)` (the device scale factor stays).
    pub async fn set_viewport_size(&self, viewport: Viewport) -> Result<()> {
        self.check_open()?;
        *self.inner.viewport.lock().unwrap() = Some(viewport);
        self.proxy_call(
            "Emulation.setDeviceMetricsOverride",
            json!({"width": viewport.width, "height": viewport.height, "fixedLayout": false,
                   "deviceScaleFactor": self.inner.options.device_scale_factor.unwrap_or(1.0)}),
        )
        .await?;
        let screen = self.inner.options.screen.unwrap_or(viewport);
        self.target_call(
            "Page.setScreenSizeOverride",
            json!({"width": screen.width, "height": screen.height}),
        )
        .await?;
        Ok(())
    }

    /// `page.viewportSize()`.
    pub fn viewport_size(&self) -> Option<Viewport> {
        Some(self.effective_viewport())
    }

    /// `page.waitForFunction(expression)`: polls until the expression (or
    /// function) is truthy.
    pub async fn wait_for_function(&self, source: &str) -> Result<()> {
        let deadline = self.inner.services.clock.now_ms() + TIMEOUT.as_millis() as u64;
        loop {
            if let Ok(v) = self.evaluate_value(source, None).await
                && truthy(&v)
            {
                return Ok(());
            }
            if self.inner.services.clock.now_ms() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for function: {source}"),
                });
            }
            self.inner
                .services
                .clock
                .sleep(Duration::from_millis(50))
                .await;
        }
    }

    /// `page.waitForTimeout(ms)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.inner.services.clock.sleep(duration).await
    }

    /// `page.screenshot()`: a builder; await it for the PNG (or JPEG).
    pub fn screenshot(&self) -> Screenshot<'_> {
        Screenshot {
            page: self,
            options: ScreenshotOptions::default(),
            path: None,
        }
    }

    async fn take_screenshot(&self, o: &ScreenshotOptions) -> Result<Vec<u8>> {
        let quality = validate(o).map_err(|e| Error::InvalidArgument(e.0))?;
        let format = match o.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Webp => {
                return Err(Error::InvalidArgument(
                    "webp screenshots are not supported on WebKit".into(),
                ));
            }
        };
        let v = self.effective_viewport();
        let viewport = Size {
            width: f64::from(v.width),
            height: f64::from(v.height),
        };
        self.call_json_in(
            &format!("({PREPARE})"),
            &[
                o.style.clone().map(Value::String).unwrap_or(Value::Null),
                json!(o.caret != Caret::Initial),
                json!(o.animations == Animations::Disabled),
                json!(o.omit_background),
            ],
            true,
        )
        .await?;
        let _ = self
            .call_json_in(
                "() => document.fonts.ready.then(() => undefined)",
                &[],
                true,
            )
            .await;
        let shot = self.capture(o, viewport, format, quality).await;
        let _ = self.call_json_in(&format!("({CLEANUP})"), &[], true).await;
        shot
    }

    async fn capture(
        &self,
        o: &ScreenshotOptions,
        viewport: Size,
        format: &str,
        quality: Option<u8>,
    ) -> Result<Vec<u8>> {
        let full_page = if o.full_page {
            let v = self
                .call_json_in(&format!("({FULL_PAGE_SIZE})"), &[], true)
                .await?;
            if v.is_null() {
                return Err(Error::Evaluation("document has no body".into()));
            }
            Some(Size {
                width: v["width"].as_f64().unwrap_or_default(),
                height: v["height"].as_f64().unwrap_or_default(),
            })
        } else {
            None
        };
        let (system, rect): (&str, Rect) =
            match plan_page(o, viewport, full_page).map_err(|e| Error::InvalidArgument(e.0))? {
                Target::Document { rect, .. } => ("Page", rect),
                Target::Viewport(r) => ("Viewport", r),
            };
        let mut params = json!({
            "x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height,
            "coordinateSystem": system,
            "omitDeviceScaleFactor": o.scale == ScreenshotScale::Css,
            "format": format,
        });
        if let Some(q) = quality {
            params["quality"] = json!(q);
        }
        let r = self.target_call("Page.snapshotRect", params).await?;
        let data = r["dataURL"].as_str().unwrap_or_default();
        let data = data.split_once(',').map_or(data, |(_, d)| d);
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| Error::Serde(format!("screenshot data: {e}")))
    }

    pub(crate) fn now(&self) -> Duration {
        Duration::from_millis(self.inner.services.clock.now_ms())
    }

    pub(crate) async fn sleep(&self, duration: Duration) {
        self.inner.services.clock.sleep(duration).await
    }

    /// Calls `body(injected, ...args)` in the utility world, installing
    /// Playwright's injected script first when needed.
    pub(crate) async fn injected_call(&self, body: &str, args: &[Value]) -> Result<Value> {
        const INJECTED: &str = "__chromiumoxide_injected";
        let call = format!(
            "(...args) => {{ const i = globalThis.{INJECTED}; \
             if (!i) return {{ __chromiumoxide_missing: true }}; return ({body})(i, ...args); }}"
        );
        let v = self.call_json_in(&call, args, true).await?;
        if v.get("__chromiumoxide_missing") != Some(&Value::Bool(true)) {
            return Ok(v);
        }
        let options = json!({
            "isUnderTest": false, "sdkLanguage": "javascript", "frameSeq": 0,
            "testIdAttributeName": "data-testid", "stableRafCount": 1, "browserName": "webkit",
            "isUtilityWorld": true, "customEngines": [],
        });
        let install = format!(
            "(() => {{ if (globalThis.{INJECTED}) return; const module = {{}};\n{}\n\
             globalThis.{INJECTED} = new (module.exports.InjectedScript())(globalThis, {options}); }})()",
            crate::application::locator::INJECTED_SOURCE
        );
        self.run(&install, true).await?;
        self.call_json_in(&call, args, true).await
    }

    /// One mouse event (`Input.dispatchMouseEvent`, viewport CSS px).
    pub(crate) async fn dispatch_mouse(&self, e: crate::domain::input::MouseEvent) -> Result<()> {
        use crate::domain::input::MouseEventType;
        let kind = match e.kind {
            MouseEventType::Moved => "move",
            MouseEventType::Pressed => "down",
            MouseEventType::Released => "up",
            MouseEventType::Wheel => {
                return self
                    .target_call("Page.updateScrollingState", json!({}))
                    .await
                    .and(Ok(()))
                    .and(
                        self.proxy_call(
                            "Input.dispatchWheelEvent",
                            json!({"x": e.x, "y": e.y, "deltaX": e.delta_x, "deltaY": e.delta_y,
                                   "modifiers": 0}),
                        )
                        .await
                        .map(drop),
                    );
            }
        };
        let modifiers =
            crate::domain::webkit::modifiers(self.inner.keyboard.lock().await.modifiers());
        let mut params = json!({"type": kind, "button": e.button, "buttons": e.buttons,
                                "x": e.x, "y": e.y, "modifiers": modifiers});
        if kind != "move" {
            params["clickCount"] = json!(e.click_count);
        }
        self.proxy_call("Input.dispatchMouseEvent", params)
            .await
            .map(drop)
    }

    /// Moves the mouse to `(x, y)` and clicks `count` times.
    pub(crate) async fn click_at(&self, x: f64, y: f64, count: u32) -> Result<()> {
        use crate::domain::input::MouseButton;
        let moves = self.inner.mouse.lock().unwrap().move_to(x, y, 1);
        for m in moves {
            self.dispatch_mouse(m).await?;
        }
        for n in 1..=count.max(1) {
            let down = self.inner.mouse.lock().unwrap().down(MouseButton::Left, n);
            self.dispatch_mouse(down).await?;
            let up = self.inner.mouse.lock().unwrap().up(MouseButton::Left, n);
            self.dispatch_mouse(up).await?;
        }
        Ok(())
    }

    /// Moves the mouse to `(x, y)`.
    pub(crate) async fn move_mouse(&self, x: f64, y: f64) -> Result<()> {
        let moves = self.inner.mouse.lock().unwrap().move_to(x, y, 1);
        for m in moves {
            self.dispatch_mouse(m).await?;
        }
        Ok(())
    }

    /// `keyboard.down(key)` / `keyboard.up(key)` (`Input.dispatchKeyEvent`).
    pub(crate) async fn key(&self, key: &str, down: bool) -> Result<()> {
        let mut keyboard = self.inner.keyboard.lock().await;
        let e = if down {
            keyboard.down(key)
        } else {
            keyboard.up(key)
        }
        .map_err(Error::InvalidArgument)?;
        drop(keyboard);
        let mut params = json!({
            "type": if down { "keyDown" } else { "keyUp" },
            "modifiers": crate::domain::webkit::modifiers(e.modifiers),
            "windowsVirtualKeyCode": e.windows_virtual_key_code,
            "code": e.code, "key": e.key, "isKeypad": e.is_keypad,
        });
        if down {
            params["text"] = json!(e.text);
            params["unmodifiedText"] = json!(e.text);
            params["autoRepeat"] = json!(e.auto_repeat);
        }
        self.proxy_call("Input.dispatchKeyEvent", params)
            .await
            .map(drop)
    }

    /// `keyboard.press(shortcut)`: `"Enter"`, `"Control+a"`, ...
    pub(crate) async fn press_shortcut(&self, shortcut: &str) -> Result<()> {
        let keys = crate::domain::input::split_shortcut(shortcut);
        for k in &keys {
            self.key(k, true).await?;
        }
        for k in keys.iter().rev() {
            self.key(k, false).await?;
        }
        Ok(())
    }

    /// `keyboard.insertText(text)` (`Page.insertText`).
    pub(crate) async fn insert_text(&self, text: &str) -> Result<()> {
        self.target_call("Page.insertText", json!({"text": text}))
            .await
            .map(drop)
    }

    /// `page.close()` (and the context `browser.new_page()` made).
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self
            .inner
            .wk
            .call(
                None,
                "Playwright.closePage",
                json!({"pageProxyId": self.inner.proxy}),
            )
            .await;
        let owned = self.inner.owned_context.lock().unwrap().take();
        if let Some(context) = owned {
            context.close().await?;
        }
        Ok(())
    }
}

/// A pending `page.screenshot()` on WebKit.
#[must_use = "does nothing until awaited"]
pub struct Screenshot<'a> {
    page: &'a Page,
    options: ScreenshotOptions,
    path: Option<PathBuf>,
}

impl<'a> Screenshot<'a> {
    pub fn full_page(mut self, yes: bool) -> Self {
        self.options.full_page = yes;
        self
    }
    pub fn clip(mut self, clip: Rect) -> Self {
        self.options.clip = Some(clip);
        self
    }
    pub fn options(mut self, options: ScreenshotOptions) -> Self {
        self.options = options;
        self
    }
    /// Also writes the image to this file.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl<'a> IntoFuture for Screenshot<'a> {
    type Output = Result<Vec<u8>>;
    type IntoFuture = BoxFuture<'a, Result<Vec<u8>>>;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let bytes = self.page.take_screenshot(&self.options).await?;
            if let Some(path) = &self.path {
                self.page.inner.services.fs.write(path, &bytes)?;
            }
            Ok(bytes)
        }
        .boxed()
    }
}

/// JavaScript truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}
