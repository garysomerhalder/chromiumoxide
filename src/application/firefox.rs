//! The Firefox engine: the Playwright API over WebDriver BiDi
//! (`chromiumoxide::firefox`). See `docs/ARCHITECTURE.md`, "Engines".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::future::IntoFuture;

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::actions::{ADD_STYLE, StyleTag};
use crate::application::firefox_download::Download;
use crate::application::firefox_events::{
    ConsoleMessage, Dialog, PageError, PageEvent, console_kind, prompt_params,
};
use crate::application::firefox_input::{InputState, Keyboard, Mouse};
use crate::application::firefox_record::{Recorders, Tracing, Video};
use crate::application::firefox_route::{Request, Route, RouteEntry};
use crate::application::firefox_worker::{WS_CHANNEL, WS_PRELOAD, Worker};
use crate::application::locator::INJECTED_SOURCE;
use crate::application::screenshot::{CLEANUP, FULL_PAGE_SIZE, PREPARE};
use crate::application::services::Services;
use crate::application::websocket::{WebSocket, WebSocketEvent, WsPayload};
use crate::domain::emulation::{ContextOptions, Viewport};
use crate::domain::error::{Error, Result};
use crate::domain::firefox::{
    command_line, cookie_from_bidi, cookie_to_bidi, init_script_function, profile_files,
    remote_value_to_json, unsupported_context_options,
};
use crate::domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale, Size, Target,
    plan_page, validate,
};
use crate::domain::storage::{
    Cookie, NameValue, NewCookie, OriginState, StorageState, filter_cookies,
};
use crate::ports::{BidiSession, BrowserProcess, FirefoxCommand};
use std::collections::HashMap;

/// The sandbox (BiDi's isolated world) for Playwright's utility scripts.
pub(crate) const UTILITY: &str = "__chromiumoxide_utility";

/// The global holding Playwright's injected script in the sandbox.
const INJECTED: &str = "__chromiumoxide_injected";

/// Options for [`crate::Firefox::launch`] (Playwright's `firefox.launch`).
#[derive(Debug, Clone)]
pub struct FirefoxLaunchOptions {
    /// The Firefox binary; `None` finds Playwright's newest Firefox
    /// (Playwright's folders, see `domain::firefox::playwright_dirs`).
    pub executable_path: Option<PathBuf>,
    /// Headless (default `true`).
    pub headless: bool,
    /// Extra command-line arguments.
    pub args: Vec<String>,
    /// Extra environment variables for the browser process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for Firefox to start (default 180 s).
    pub timeout: Duration,
}

impl Default for FirefoxLaunchOptions {
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

/// The newest `firefox-<revision>/firefox/firefox` Playwright installed.
fn find_firefox(services: &Services) -> Option<PathBuf> {
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
        "firefox/firefox.exe"
    } else if cfg!(target_os = "macos") {
        "firefox/Nightly.app/Contents/MacOS/firefox"
    } else {
        "firefox/firefox"
    };
    let mut found: Vec<(u32, PathBuf)> = dirs
        .iter()
        .flat_map(|d| {
            services
                .fs
                .list_dir(d)
                .into_iter()
                .filter_map(|name| {
                    let rev = name.strip_prefix("firefox-")?.parse().ok()?;
                    let path = d.join(&name).join(exe);
                    services.fs.is_file(&path).then_some((rev, path))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// Opens the BiDi session (prompts stay open until the page's dialog
/// handling answers them) and subscribes to the engine's events; the
/// browser version.
async fn open_session(bidi: &Arc<dyn BidiSession>) -> Result<String> {
    let session = bidi
        .call(
            "session.new",
            json!({"capabilities": {"alwaysMatch": {"acceptInsecureCerts": false,
                "unhandledPromptBehavior": {"default": "ignore"}}}}),
        )
        .await?;
    bidi.call(
        "session.subscribe",
        json!({"events": ["browsingContext", "log.entryAdded", "network",
                           "script.realmCreated", "script.realmDestroyed", "script.message",
                           "input.fileDialogOpened"]}),
    )
    .await?;
    Ok(session
        .pointer("/capabilities/browserVersion")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// A Firefox browser driven over WebDriver BiDi.
#[derive(Clone)]
pub struct Browser {
    pub(crate) inner: Arc<BrowserInner>,
}

pub(crate) struct BrowserInner {
    bidi: Arc<dyn BidiSession>,
    services: Services,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
    /// The temporary profile (removed on close).
    profile: Option<PathBuf>,
    version: String,
    /// User contexts made by this client (removed when a connected
    /// browser closes).
    created: Mutex<Vec<String>>,
    /// The contexts made through this object (`browser.contexts()`).
    pub(crate) contexts: Mutex<Vec<std::sync::Weak<ContextInner>>>,
    pub(crate) closed: AtomicBool,
}

impl std::fmt::Debug for Browser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Browser")
            .field("version", &self.inner.version)
            .finish()
    }
}

impl Browser {
    /// Starts Firefox and opens a BiDi session.
    pub async fn launch(services: Services, options: FirefoxLaunchOptions) -> Result<Browser> {
        Self::launch_in(services, options, None).await
    }

    /// `firefox.launchPersistentContext(userDataDir, options)`: Firefox on
    /// a profile that is kept (created when missing); the context is the
    /// default user context, with the window Firefox opened, and closing
    /// it closes Firefox.
    pub async fn launch_persistent_context(
        services: Services,
        user_data_dir: &std::path::Path,
        options: FirefoxLaunchOptions,
        context_options: ContextOptions,
    ) -> Result<BrowserContext> {
        services
            .fs
            .create_dir_all(user_data_dir)
            .map_err(|e| Error::Launch(format!("cannot create the profile: {e}")))?;
        let browser = Self::launch_in(services, options, Some(user_data_dir.to_owned())).await?;
        let events = browser.inner.bidi.subscribe();
        let context = match browser
            .make_context("default".to_owned(), context_options, Some(browser.clone()))
            .await
        {
            Ok(c) => c,
            Err(e) => {
                let _ = browser.close().await;
                return Err(e);
            }
        };
        let tree = browser
            .inner
            .bidi
            .call("browsingContext.getTree", json!({"maxDepth": 0}))
            .await?;
        let mut events = Some(events);
        for top in tree["contexts"].as_array().into_iter().flatten() {
            if top["userContext"] != "default" {
                continue;
            }
            let events = events
                .take()
                .unwrap_or_else(|| browser.inner.bidi.subscribe());
            let page = attach_page(
                &context.inner,
                top["context"].as_str().unwrap_or_default().to_owned(),
                top["url"].as_str().unwrap_or("about:blank").to_owned(),
                events,
            )
            .await?;
            page.refresh_url().await;
        }
        Ok(context)
    }

    /// `firefox.connect(wsEndpoint)` over an open BiDi connection: a new
    /// session on a Firefox started elsewhere. `close` then removes the
    /// contexts made here and ends the session; Firefox keeps running.
    pub async fn connect(services: Services, bidi: Arc<dyn BidiSession>) -> Result<Browser> {
        let version = match open_session(&bidi).await {
            Ok(v) => v,
            Err(e) => {
                bidi.disconnect();
                return Err(e);
            }
        };
        Ok(Browser {
            inner: Arc::new(BrowserInner {
                bidi,
                services,
                process: Mutex::new(None),
                profile: None,
                version,
                created: Mutex::new(Vec::new()),
                contexts: Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
            }),
        })
    }

    async fn launch_in(
        services: Services,
        options: FirefoxLaunchOptions,
        user_data_dir: Option<PathBuf>,
    ) -> Result<Browser> {
        let program = match options.executable_path.clone() {
            Some(p) => p,
            None => find_firefox(&services).ok_or_else(|| {
                Error::Launch(
                    "no Firefox found: install Playwright's (`npx playwright install firefox`) \
                     or set executable_path"
                        .into(),
                )
            })?,
        };
        let keep_profile = user_data_dir.is_some();
        let profile = match user_data_dir {
            Some(dir) => dir,
            None => services
                .fs
                .create_temp_dir("chromiumoxide-firefox-profile-")
                .map_err(|e| Error::Launch(format!("cannot create the profile: {e}")))?,
        };
        // Only a temporary profile is removed on failure or close.
        let remove_profile = |services: &Services| {
            if !keep_profile {
                let _ = services.fs.remove_dir_all(&profile);
            }
        };
        for (path, contents) in profile_files(options.headless) {
            let path = profile.join(path);
            let written = match path.parent() {
                Some(dir) => services.fs.create_dir_all(dir),
                None => Ok(()),
            }
            .and_then(|()| services.fs.write(&path, contents.as_bytes()));
            if let Err(e) = written {
                remove_profile(&services);
                return Err(Error::Launch(format!("cannot write the profile: {e}")));
            }
        }
        let mut env = vec![("MOZ_DISABLE_SAFE_MODE_KEY".to_owned(), "1".to_owned())];
        env.extend(options.env.iter().cloned());
        let launched = services
            .launcher
            .launch_firefox(FirefoxCommand {
                program,
                args: command_line(&profile, options.headless, &options.args),
                env,
                timeout_ms: options.timeout.as_millis() as u64,
            })
            .await;
        let launched = match launched {
            Ok(l) => l,
            Err(e) => {
                remove_profile(&services);
                return Err(e);
            }
        };
        let version = match open_session(&launched.bidi).await {
            Ok(v) => v,
            Err(e) => {
                launched.process.shutdown().await;
                remove_profile(&services);
                return Err(e);
            }
        };
        Ok(Browser {
            inner: Arc::new(BrowserInner {
                bidi: launched.bidi,
                services,
                process: Mutex::new(Some(launched.process)),
                profile: (!keep_profile).then_some(profile),
                version,
                created: Mutex::new(Vec::new()),
                contexts: Mutex::new(Vec::new()),
                closed: AtomicBool::new(false),
            }),
        })
    }

    /// `browser.version()`, e.g. `159.0`.
    pub async fn version(&self) -> Result<String> {
        Ok(self.inner.version.clone())
    }

    /// `browser.newContext(options)`: a BiDi user context with the
    /// options' emulation. Options Firefox cannot honour over BiDi are an
    /// error (see `domain::firefox::unsupported_context_options`).
    pub async fn new_context(&self, options: ContextOptions) -> Result<BrowserContext> {
        let unsupported = unsupported_context_options(&options);
        if !unsupported.is_empty() {
            return Err(Error::InvalidArgument(format!(
                "not supported on Firefox (no WebDriver BiDi command in Firefox {}): {}",
                self.inner.version,
                unsupported.join(", ")
            )));
        }
        let mut create = json!({});
        if options.ignore_https_errors {
            create["acceptInsecureCerts"] = json!(true);
        }
        let user_context = self
            .inner
            .bidi
            .call("browser.createUserContext", create)
            .await?
            .get("userContext")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        self.inner
            .created
            .lock()
            .unwrap()
            .push(user_context.clone());
        self.make_context(user_context, options, None).await
    }

    /// The context object of a user context, with its emulation, storage
    /// state, recorders and WebSocket reporting set up. `persistent` is the
    /// browser a persistent context closes.
    async fn make_context(
        &self,
        user_context: String,
        options: ContextOptions,
        persistent: Option<Browser>,
    ) -> Result<BrowserContext> {
        let unsupported = unsupported_context_options(&options);
        if !unsupported.is_empty() {
            return Err(Error::InvalidArgument(format!(
                "not supported on Firefox (no WebDriver BiDi command in Firefox {}): {}",
                self.inner.version,
                unsupported.join(", ")
            )));
        }
        let bidi = &self.inner.bidi;
        let options_accept_downloads = options.accept_downloads;
        let record_har = options.record_har.clone();
        let viewport = options
            .viewport
            .map_or((1280, 720), |v| (v.width, v.height));
        let extra_headers: Vec<(String, String)> = options
            .extra_http_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let credentials = options.http_credentials.clone();
        let permissions = options.permissions.clone();
        let context = BrowserContext {
            inner: Arc::new(ContextInner {
                bidi: bidi.clone(),
                services: self.inner.services.clone(),
                user_context: user_context.clone(),
                options: Arc::new(options),
                routes: Arc::default(),
                pages: Mutex::new(Vec::new()),
                videos: Mutex::new(Vec::new()),
                downloads_dir: if options_accept_downloads {
                    self.inner
                        .services
                        .fs
                        .create_temp_dir("chromiumoxide-firefox-downloads-")
                        .ok()
                } else {
                    None
                },
                recorders: Arc::new(Recorders::new(
                    bidi.clone(),
                    self.inner.services.clone(),
                    user_context.clone(),
                    self.inner.version.clone(),
                    viewport,
                )),
                persistent: Mutex::new(persistent),
                timeouts: Mutex::new(Timeouts::default()),
                paused: tokio::sync::watch::channel(false).0,
                browser: Arc::downgrade(&self.inner),
                extra_headers: Mutex::new(extra_headers),
                credentials: Mutex::new(credentials),
                granted_any: Mutex::new(permissions),
                granted: Mutex::new(Vec::new()),
                subscribers: Mutex::new(Vec::new()),
                clock_installed: AtomicBool::new(false),
                closed: AtomicBool::new(false),
            }),
        };
        if let Some(har) = record_har
            && let Err(e) = context.inner.recorders.start_har(har).await
        {
            let _ = context.close().await;
            return Err(e);
        }
        if let Err(e) = bidi
            .call(
                "script.addPreloadScript",
                json!({"functionDeclaration": WS_PRELOAD,
                       "arguments": [{"type": "channel", "value": {"channel": WS_CHANNEL}}],
                       "userContexts": [user_context]}),
            )
            .await
        {
            let _ = context.close().await;
            return Err(e);
        }
        self.inner
            .contexts
            .lock()
            .unwrap()
            .push(Arc::downgrade(&context.inner));
        // Keeps request and response bodies (`request.postData()`,
        // `response.body()`).
        if let Err(e) = context.inner.recorders.ensure_collector().await {
            let _ = context.close().await;
            return Err(e);
        }
        if let Err(e) = context.apply_emulation().await {
            let _ = context.close().await;
            return Err(e);
        }
        if let Some(state) = context.inner.options.storage_state.clone()
            && let Err(e) = context.restore_storage_state(&state).await
        {
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

    /// `browser.close()`: closes Firefox and deletes its temporary profile.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let process = self.inner.process.lock().unwrap().take();
        if process.is_none() {
            // Connected: leave Firefox running, without our contexts.
            let created: Vec<String> = self.inner.created.lock().unwrap().drain(..).collect();
            for user_context in created {
                let _ = self
                    .inner
                    .bidi
                    .call(
                        "browser.removeUserContext",
                        json!({"userContext": user_context}),
                    )
                    .await;
            }
            let _ = self.inner.bidi.call("session.end", json!({})).await;
            self.inner.bidi.disconnect();
            return Ok(());
        }
        let _ = self.inner.bidi.call("browser.close", json!({})).await;
        if let Some(process) = process {
            process.shutdown().await;
        }
        if let Some(profile) = &self.inner.profile {
            let _ = self.inner.services.fs.remove_dir_all(profile);
        }
        Ok(())
    }
}

/// A Firefox browser context (a BiDi user context).
#[derive(Clone)]
pub struct BrowserContext {
    pub(crate) inner: Arc<ContextInner>,
}

pub(crate) struct ContextInner {
    pub(crate) bidi: Arc<dyn BidiSession>,
    pub(crate) services: Services,
    pub(crate) user_context: String,
    pub(crate) options: Arc<ContextOptions>,
    /// `context.route` handlers, shared with the context's pages.
    routes: Arc<Mutex<Vec<RouteEntry>>>,
    /// The context's pages (to add network intercepts for new routes).
    /// Held until they close, like Playwright's pages.
    pages: Mutex<Vec<Arc<PageInner>>>,
    /// Videos of the context's pages (`close` waits for them).
    videos: Mutex<Vec<Video>>,
    /// Where this context's downloads go (removed with the context).
    downloads_dir: Option<PathBuf>,
    /// HAR and tracing recorders (shared with the pages).
    recorders: Arc<Recorders>,
    /// The browser of a persistent context (closed with it).
    persistent: Mutex<Option<Browser>>,
    /// `context.setDefaultTimeout` / `setDefaultNavigationTimeout`.
    pub(crate) timeouts: Mutex<Timeouts>,
    /// `page.pause()` waits while this is true; `context.resume()` clears it.
    pub(crate) paused: tokio::sync::watch::Sender<bool>,
    /// The browser object that made this context.
    pub(crate) browser: std::sync::Weak<BrowserInner>,
    /// `context.setExtraHTTPHeaders` (starts as the option).
    pub(crate) extra_headers: Mutex<Vec<(String, String)>>,
    /// `context.setHTTPCredentials` (starts as the option).
    pub(crate) credentials: Mutex<Option<crate::domain::emulation::HttpCredentials>>,
    /// Permissions granted for any origin (applied to each origin a page
    /// navigates to) and the `(origin, name)` pairs set so far.
    pub(crate) granted_any: Mutex<Vec<String>>,
    pub(crate) granted: Mutex<Vec<(String, String)>>,
    /// Playwright's clock script is in the context's pages.
    pub(crate) clock_installed: AtomicBool,
    /// `context.events()` subscribers.
    pub(crate) subscribers: Mutex<
        Vec<
            futures::channel::mpsc::UnboundedSender<
                crate::application::firefox_events::ContextEvent,
            >,
        >,
    >,
    pub(crate) closed: AtomicBool,
}

/// Default timeouts set on a page or context (`None`: not set there).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Timeouts {
    pub(crate) action: Option<Duration>,
    pub(crate) navigation: Option<Duration>,
}

/// Playwright's default timeout.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// What a page remembers for `consoleMessages()`, `pageErrors()` and
/// `requests()` (Playwright keeps the last 200, 200 and 100).
#[derive(Default)]
pub(crate) struct History {
    pub(crate) console: std::collections::VecDeque<ConsoleMessage>,
    pub(crate) errors: std::collections::VecDeque<PageError>,
    pub(crate) requests: std::collections::VecDeque<Request>,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::BrowserContext")
            .field("user_context", &self.inner.user_context)
            .finish()
    }
}

impl ContextInner {
    /// Sends an event to `context.events()` subscribers; false if none.
    pub(crate) fn emit(&self, event: crate::application::firefox_events::ContextEvent) -> bool {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|s| s.unbounded_send(event.clone()).is_ok());
        !subs.is_empty()
    }
}

impl BrowserContext {
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

    pub(crate) fn from_inner(inner: Arc<ContextInner>) -> BrowserContext {
        BrowserContext { inner }
    }
}

impl BrowserContext {
    /// The context's emulation, set once for the user context so every
    /// page in it starts emulated.
    async fn apply_emulation(&self) -> Result<()> {
        let o = &self.inner.options;
        let bidi = &self.inner.bidi;
        let ucs = json!([self.inner.user_context]);
        if o.viewport.is_some() || o.device_scale_factor.is_some() {
            let mut p = json!({"userContexts": ucs});
            if let Some(v) = o.viewport {
                p["viewport"] = json!({"width": v.width, "height": v.height});
            }
            if let Some(dpr) = o.device_scale_factor {
                p["devicePixelRatio"] = json!(dpr);
            }
            bidi.call("browsingContext.setViewport", p).await?;
        }
        if let Some(ua) = &o.user_agent {
            bidi.call(
                "emulation.setUserAgentOverride",
                json!({"userAgent": ua, "userContexts": ucs}),
            )
            .await?;
        }
        if let Some(locale) = &o.locale {
            bidi.call(
                "emulation.setLocaleOverride",
                json!({"locale": locale, "userContexts": ucs}),
            )
            .await?;
        }
        if let Some(tz) = &o.timezone_id {
            bidi.call(
                "emulation.setTimezoneOverride",
                json!({"timezone": tz, "userContexts": ucs}),
            )
            .await?;
        }
        if let Some(screen) = o.screen {
            bidi.call(
                "emulation.setScreenSettingsOverride",
                json!({"screenArea": {"width": screen.width, "height": screen.height}, "userContexts": ucs}),
            )
            .await?;
        }
        if let Some(g) = o.geolocation {
            bidi.call(
                "emulation.setGeolocationOverride",
                json!({"coordinates": {"latitude": g.latitude, "longitude": g.longitude, "accuracy": g.accuracy},
                       "userContexts": ucs}),
            )
            .await?;
        }
        let behavior = match &self.inner.downloads_dir {
            Some(dir) => json!({"type": "allowed", "destinationFolder": dir.display().to_string()}),
            None => json!({"type": "denied"}),
        };
        bidi.call(
            "browser.setDownloadBehavior",
            json!({"downloadBehavior": behavior, "userContexts": ucs}),
        )
        .await?;
        if o.offline {
            bidi.call(
                "emulation.setNetworkConditions",
                json!({"networkConditions": {"type": "offline"}, "userContexts": ucs}),
            )
            .await?;
        }
        if !o.extra_http_headers.is_empty() {
            let headers: Vec<Value> = o
                .extra_http_headers
                .iter()
                .map(|(k, v)| json!({"name": k, "value": {"type": "string", "value": v}}))
                .collect();
            bidi.call(
                "network.setExtraHeaders",
                json!({"headers": headers, "userContexts": ucs}),
            )
            .await?;
        }
        Ok(())
    }

    pub(crate) fn live_pages(&self) -> Vec<Page> {
        let mut pages = self.inner.pages.lock().unwrap();
        pages.retain(|p| !p.closed.load(Ordering::SeqCst));
        pages
            .iter()
            .map(|inner| Page {
                inner: inner.clone(),
            })
            .collect()
    }

    /// Registers a context route; every page gets its network intercept.
    pub(crate) async fn add_route(&self, entry: RouteEntry) -> Result<()> {
        self.inner.routes.lock().unwrap().push(entry);
        for page in self.live_pages() {
            page.sync_intercept().await?;
        }
        Ok(())
    }

    /// Removes context routes (and intercepts no page needs any more).
    pub(crate) async fn remove_routes(&self, remove: impl Fn(&RouteEntry) -> bool) -> Result<()> {
        self.inner.routes.lock().unwrap().retain(|e| !remove(e));
        for page in self.live_pages() {
            page.sync_intercept().await?;
        }
        Ok(())
    }

    /// `context.storageState()`: the cookies, and the localStorage of the
    /// origins open in this context's pages.
    pub async fn storage_state(&self) -> Result<StorageState> {
        let cookies = self.cookies().await?;
        let mut origins: Vec<OriginState> = Vec::new();
        for page in self.live_pages() {
            let v = page
                .call_json(
                    "() => ({ origin: location.origin, items: Object.keys(localStorage)\
                     .map(name => ({ name, value: localStorage.getItem(name) })) })",
                    &[],
                    None,
                )
                .await;
            let Ok(v) = v else {
                continue;
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

    /// Playwright's way to restore `storageState`: the cookies, then per
    /// origin a temporary page served a blank document (the site is never
    /// loaded) that writes the localStorage.
    pub(crate) async fn restore_storage_state(&self, state: &StorageState) -> Result<()> {
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
                page.goto(&format!("{}/", origin.origin.trim_end_matches('/'))).await?;
                page.call_json(
                    "(items) => { for (const { name, value } of items) localStorage.setItem(name, value); }",
                    &[json!(items)],
                    None,
                )
                .await
            }
            .await;
            page.close().await?;
            written?;
        }
        Ok(())
    }

    pub(crate) fn partition(&self) -> Value {
        json!({"type": "storageKey", "userContext": self.inner.user_context})
    }

    /// `context.cookies()`: every cookie of this context.
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        self.cookies_for(&[]).await
    }

    /// `context.cookies(urls)`: the cookies sent to any of `urls`.
    pub async fn cookies_for(&self, urls: &[&str]) -> Result<Vec<Cookie>> {
        let r = self
            .inner
            .bidi
            .call("storage.getCookies", json!({"partition": self.partition()}))
            .await?;
        let cookies = r
            .get("cookies")
            .and_then(Value::as_array)
            .map(|list| list.iter().map(cookie_from_bidi).collect())
            .unwrap_or_default();
        Ok(filter_cookies(cookies, urls))
    }

    /// `context.addCookies(cookies)` (checked like Playwright's).
    pub async fn add_cookies(&self, cookies: &[NewCookie]) -> Result<()> {
        let resolved = cookies
            .iter()
            .map(NewCookie::resolve)
            .collect::<std::result::Result<Vec<_>, String>>()
            .map_err(Error::InvalidArgument)?;
        for c in &resolved {
            self.inner
                .bidi
                .call(
                    "storage.setCookie",
                    json!({"cookie": cookie_to_bidi(c), "partition": self.partition()}),
                )
                .await?;
        }
        Ok(())
    }

    /// `context.clearCookies()`.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.inner
            .bidi
            .call(
                "storage.deleteCookies",
                json!({"partition": self.partition()}),
            )
            .await?;
        Ok(())
    }

    /// `context.addInitScript(source)`: runs before any page script in
    /// every document of this context.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.inner
            .bidi
            .call(
                "script.addPreloadScript",
                json!({"functionDeclaration": init_script_function(source),
                       "userContexts": [self.inner.user_context]}),
            )
            .await?;
        Ok(())
    }

    /// `context.newPage()`: a new window in this user context.
    pub async fn new_page(&self) -> Result<Page> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("context has been closed".into()));
        }
        let events = self.inner.bidi.subscribe();
        let created = self
            .inner
            .bidi
            .call(
                "browsingContext.create",
                // A foreground tab never finishes opening in headless
                // Firefox (it waits for focus); a window does, like
                // Playwright's one window per page.
                json!({"type": "window", "userContext": self.inner.user_context}),
            )
            .await?;
        let context = created
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        attach_page(&self.inner, context, "about:blank".to_owned(), events).await
    }

    /// `context.pages()`: the context's open pages.
    pub fn pages(&self) -> Vec<Page> {
        self.live_pages()
    }

    /// `context.close()`: closes its pages and the user context.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner
            .emit(crate::application::firefox_events::ContextEvent::Close);
        let har = self.inner.recorders.flush_har(&self.live_pages()).await;
        let pages = self.live_pages();
        let persistent = self.inner.persistent.lock().unwrap().take();
        match &persistent {
            // The default user context cannot be removed: Firefox closes.
            Some(browser) => browser.close().await?,
            None => {
                self.inner
                    .bidi
                    .call(
                        "browser.removeUserContext",
                        json!({"userContext": self.inner.user_context}),
                    )
                    .await?;
            }
        }
        if let Some(dir) = &self.inner.downloads_dir {
            let _ = self.inner.services.fs.remove_dir_all(dir);
        }
        // Like Playwright, close resolves once the pages' videos are written.
        drop(pages);
        let videos: Vec<Video> = self.inner.videos.lock().unwrap().clone();
        for video in videos {
            let _ = video.path().await;
        }
        self.inner.pages.lock().unwrap().clear();
        har
    }

    /// `context.tracing()`.
    pub fn tracing(&self) -> Tracing {
        Tracing::new(self.inner.recorders.clone(), self.live_pages())
    }
}

/// Makes the page object of a top-level browsing context of `ctx` (a new
/// page or a pop-up) and starts following its events (`events` was
/// subscribed before the context could send any).
async fn attach_page(
    ctx: &Arc<ContextInner>,
    context: String,
    url: String,
    mut events: crate::ports::EventStream,
) -> Result<Page> {
    let inner = Arc::new(PageInner {
        bidi: ctx.bidi.clone(),
        services: ctx.services.clone(),
        context: context.clone(),
        options: ctx.options.clone(),
        viewport: Mutex::new(None),
        owned_context: Mutex::new(None),
        url: Mutex::new(url),
        subscribers: Mutex::new(Vec::new()),
        routes: Mutex::new(Vec::new()),
        context_routes: ctx.routes.clone(),
        auth_attempts: Mutex::new(std::collections::HashSet::new()),
        downloads: Mutex::new(HashMap::new()),
        input: Arc::default(),
        recorders: ctx.recorders.clone(),
        video: Mutex::new(None),
        intercept: tokio::sync::Mutex::new(None),
        owner: Arc::downgrade(ctx),
        realms: Mutex::new(std::collections::HashSet::new()),
        workers: Mutex::new(Vec::new()),
        websockets: Mutex::new(HashMap::new()),
        history: Mutex::new(History::default()),
        opener: Mutex::new(None),
        timeouts: Mutex::new(Timeouts::default()),
        picking: AtomicBool::new(false),
        extra_headers: Mutex::new(Vec::new()),
        requests: Mutex::new(HashMap::new()),
        pending_post: Mutex::new(HashMap::new()),
        closed: AtomicBool::new(false),
    });
    ctx.pages.lock().unwrap().push(inner.clone());
    let weak = Arc::downgrade(&inner);
    tokio::spawn(async move {
        use futures::StreamExt;
        while let Some(event) = events.next().await {
            let event_context = event
                .params
                .get("context")
                .or_else(|| event.params.pointer("/source/context"))
                .and_then(Value::as_str);
            let Some(page) = weak.upgrade() else {
                break;
            };
            let page = Page { inner: page };
            if event_context != Some(context.as_str()) {
                page.foreign_event(&event.method, &event.params);
                continue;
            }
            if event.method == "network.beforeRequestSent"
                && event
                    .params
                    .pointer("/request/bodySize")
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
                    > 0
            {
                // Read the body before the request is reported, so
                // `request.postData()` has it.
                let id = event
                    .params
                    .pointer("/request/request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if let Some(body) = page.collected_data(&id, "request").await {
                    page.inner.pending_post.lock().unwrap().insert(id, body);
                }
            }
            page.dispatch(&event.method, &event.params);
            page.inner
                .recorders
                .on_event(&page, &event.method, &event.params);
            let page = page.inner;
            match event.method.as_str() {
                "browsingContext.navigationCommitted"
                | "browsingContext.fragmentNavigated"
                | "browsingContext.historyUpdated" => {
                    if let Some(url) = event.params.get("url").and_then(Value::as_str) {
                        *page.url.lock().unwrap() = url.to_owned();
                    }
                }
                "browsingContext.contextDestroyed" => {
                    page.closed.store(true, Ordering::SeqCst);
                    for ws in page.websockets.lock().unwrap().drain() {
                        ws.1.push(WebSocketEvent::Close);
                    }
                    for w in page.workers.lock().unwrap().drain(..) {
                        w.mark_closed();
                    }
                    page.subscribers.lock().unwrap().clear();
                    break;
                }
                _ => {}
            }
        }
    });
    let page = Page { inner };
    let video = Video::start(&page);
    if let Some(v) = &video {
        ctx.videos.lock().unwrap().push(v.clone());
    }
    *page.inner.video.lock().unwrap() = video;
    ctx.recorders.page_created(&page);
    page.sync_intercept().await?;
    ctx.emit(crate::application::firefox_events::ContextEvent::Page(
        page.clone(),
    ));
    if ctx.credentials.lock().unwrap().is_some() {
        page.inner
            .bidi
            .call(
                "network.addIntercept",
                json!({"phases": ["authRequired"], "contexts": [page.inner.context]}),
            )
            .await?;
    }
    Ok(page)
}

/// A Firefox page (a top-level browsing context in its own window).
#[derive(Clone)]
pub struct Page {
    pub(crate) inner: Arc<PageInner>,
}

pub struct PageInner {
    bidi: Arc<dyn BidiSession>,
    pub(crate) services: Services,
    /// The browsing context id.
    context: String,
    options: Arc<ContextOptions>,
    /// `set_viewport_size` (else the context's viewport).
    viewport: Mutex<Option<Viewport>>,
    /// The context `browser.new_page()` made for this page.
    owned_context: Mutex<Option<BrowserContext>>,
    /// The main frame's URL, from `browsingContext` events.
    url: Mutex<String>,
    /// `page.route` handlers, oldest first.
    routes: Mutex<Vec<RouteEntry>>,
    /// The context's `context.route` handlers (shared).
    context_routes: Arc<Mutex<Vec<RouteEntry>>>,
    /// Keyboard and mouse state (`page.keyboard()`, `page.mouse()`).
    input: Arc<InputState>,
    /// Downloads in progress, by BiDi navigation id.
    downloads: Mutex<HashMap<String, Download>>,
    /// Requests already answered with credentials (a second challenge is
    /// cancelled, like Playwright).
    auth_attempts: Mutex<std::collections::HashSet<String>>,
    /// The BiDi network intercept while routes exist.
    intercept: tokio::sync::Mutex<Option<String>>,
    /// The context's recorders.
    recorders: Arc<Recorders>,
    /// `page.video()`.
    video: Mutex<Option<Video>>,
    /// The context the page belongs to.
    pub(crate) owner: std::sync::Weak<ContextInner>,
    /// The page's window realms (owners of its dedicated workers).
    realms: Mutex<std::collections::HashSet<String>>,
    /// `page.workers()`.
    workers: Mutex<Vec<Worker>>,
    /// Open WebSockets, by `<realm>:<id>`.
    websockets: Mutex<HashMap<String, WebSocket>>,
    /// `page.events()` subscribers.
    subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<PageEvent>>>,
    /// Recent console messages, errors and requests.
    pub(crate) history: Mutex<History>,
    /// The page that opened this one (`page.opener()`).
    pub(crate) opener: Mutex<Option<Page>>,
    /// `page.setDefaultTimeout` / `setDefaultNavigationTimeout`.
    pub(crate) timeouts: Mutex<Timeouts>,
    /// While `page.pickLocator()` runs.
    pub(crate) picking: AtomicBool,
    /// `page.setExtraHTTPHeaders`.
    pub(crate) extra_headers: Mutex<Vec<(String, String)>>,
    /// Requests in flight, by BiDi request id (the newest hop).
    pub(crate) requests: Mutex<HashMap<String, Request>>,
    /// Request bodies read before their `beforeRequestSent` is handled.
    pub(crate) pending_post: Mutex<HashMap<String, Vec<u8>>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Page")
            .field("context", &self.inner.context)
            .finish()
    }
}

impl Page {
    /// The action timeout: the page's, else the context's, else 30 s.
    pub(crate) fn timeout(&self) -> Duration {
        let own = self.inner.timeouts.lock().unwrap().action;
        own.or_else(|| {
            self.inner
                .owner
                .upgrade()
                .and_then(|c| c.timeouts.lock().unwrap().action)
        })
        .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// The navigation timeout: the page's navigation, else its default,
    /// else the context's navigation, else the context's default, else 30 s.
    pub(crate) fn navigation_timeout(&self) -> Duration {
        let own = *self.inner.timeouts.lock().unwrap();
        let ctx = self
            .inner
            .owner
            .upgrade()
            .map(|c| *c.timeouts.lock().unwrap())
            .unwrap_or_default();
        own.navigation
            .or(own.action)
            .or(ctx.navigation)
            .or(ctx.action)
            .unwrap_or(DEFAULT_TIMEOUT)
    }

    /// The live request a BiDi `network` event is about (a new one, if it
    /// was not seen).
    fn request_of(&self, p: &Value) -> Request {
        let id = p
            .pointer("/request/request")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if let Some(r) = self.inner.requests.lock().unwrap().get(id) {
            return r.clone();
        }
        Request::from_event(p, Arc::downgrade(&self.inner), None, None)
    }

    /// A request's or response's body from the context's data collector.
    pub(crate) async fn collected_data(&self, request: &str, data_type: &str) -> Option<Vec<u8>> {
        let collector = self.inner.recorders.collector_id().await?;
        crate::application::firefox_record::collected(
            &self.inner.bidi,
            &collector,
            request,
            data_type,
        )
        .await
    }

    /// `page.isClosed()`.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("page has been closed".into()));
        }
        Ok(())
    }

    /// `page.goto(url)`: navigates and waits for the `load` event.
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.traced(
            "Frame",
            "goto",
            serde_json::json!({"url": url, "waitUntil": "load"}),
            self.goto_untraced(url),
        )
        .await
    }

    async fn goto_untraced(&self, url: &str) -> Result<()> {
        let context = self.inner.context.clone();
        let timeout = self.navigation_timeout();
        crate::application::wait::with_timeout(
            &*self.inner.services.clock,
            timeout,
            &format!("navigating to \"{url}\""),
            self.goto_in(&context, url),
        )
        .await
    }

    /// Navigates a frame's browsing context and waits for its `load`.
    pub(crate) async fn goto_in(&self, context: &str, url: &str) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "browsingContext.navigate",
                json!({"context": context, "wait": "complete",
                       "url": crate::domain::url_matcher::resolve_url(
                           self.inner.options.base_url.as_deref(), url)}),
            )
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        Ok(())
    }

    /// `page.events()`: console messages, errors, dialogs, load and close
    /// from now on. While nobody listens, dialogs are dismissed (like
    /// Playwright with no `dialog` handler).
    pub fn events(&self) -> impl futures::Stream<Item = PageEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// Sends an event to the live subscribers; false when there are none.
    fn emit(&self, event: PageEvent) -> bool {
        {
            let mut h = self.inner.history.lock().unwrap();
            match &event {
                PageEvent::Console(m) => {
                    h.console.push_back(m.clone());
                    if h.console.len() > 200 {
                        h.console.pop_front();
                    }
                }
                PageEvent::PageError(e) => {
                    h.errors.push_back(e.clone());
                    if h.errors.len() > 200 {
                        h.errors.pop_front();
                    }
                }
                PageEvent::Request(r) => {
                    h.requests.push_back(r.clone());
                    if h.requests.len() > 100 {
                        h.requests.pop_front();
                    }
                }
                _ => {}
            }
        }
        let context_listened = match (
            crate::application::firefox_events::ContextEvent::from_page(&event),
            self.inner.owner.upgrade(),
        ) {
            (Some(e), Some(ctx)) => ctx.emit(e),
            _ => false,
        };
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.retain(|s| s.unbounded_send(event.clone()).is_ok());
        !subs.is_empty() || context_listened
    }

    /// Turns one BiDi event for this page into page events.
    fn dispatch(&self, method: &str, p: &Value) {
        let text = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match method {
            "log.entryAdded" if p["type"] == "console" => {
                let frame = p.pointer("/stackTrace/callFrames/0");
                self.emit(PageEvent::Console(ConsoleMessage {
                    kind: console_kind(p["method"].as_str().unwrap_or("log")),
                    text: text("text"),
                    args: p["args"].as_array().cloned().unwrap_or_default(),
                    timestamp: p["timestamp"].as_f64().unwrap_or_default(),
                    location: crate::domain::console::Location {
                        url: frame
                            .and_then(|f| f["url"].as_str())
                            .unwrap_or_default()
                            .to_owned(),
                        line_number: frame
                            .and_then(|f| f["lineNumber"].as_u64())
                            .unwrap_or_default() as u32,
                        column_number: frame
                            .and_then(|f| f["columnNumber"].as_u64())
                            .unwrap_or_default() as u32,
                    },
                    context: p
                        .pointer("/source/context")
                        .and_then(Value::as_str)
                        .unwrap_or(&self.inner.context)
                        .to_owned(),
                    page: Arc::downgrade(&self.inner),
                }));
            }
            "log.entryAdded" if p["type"] == "javascript" => {
                let stack = p
                    .pointer("/stackTrace/callFrames")
                    .and_then(Value::as_array)
                    .map(|frames| {
                        frames
                            .iter()
                            .map(|f| {
                                format!(
                                    "    at {} ({}:{}:{})",
                                    f["functionName"].as_str().unwrap_or_default(),
                                    f["url"].as_str().unwrap_or_default(),
                                    f["lineNumber"].as_i64().unwrap_or_default() + 1,
                                    f["columnNumber"].as_i64().unwrap_or_default() + 1
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                self.emit(PageEvent::PageError(PageError::from_text(
                    &text("text"),
                    stack,
                )));
            }
            "browsingContext.userPromptOpened" => {
                let dialog = Dialog {
                    page: self.clone(),
                    kind: text("type"),
                    message: text("message"),
                    default_value: text("defaultValue"),
                };
                if !self.emit(PageEvent::Dialog(dialog)) {
                    let page = self.clone();
                    tokio::spawn(async move {
                        let _ = page.handle_user_prompt(false, None).await;
                    });
                }
            }
            "network.beforeRequestSent" => {
                let id = p
                    .pointer("/request/request")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let previous = (p["redirectCount"].as_u64().unwrap_or_default() > 0)
                    .then(|| self.inner.requests.lock().unwrap().get(&id).cloned())
                    .flatten();
                let post = self.inner.pending_post.lock().unwrap().remove(&id);
                let request =
                    Request::from_event(p, Arc::downgrade(&self.inner), previous.clone(), post);
                if let Some(previous) = previous {
                    previous.redirected(&request);
                }
                self.inner
                    .requests
                    .lock()
                    .unwrap()
                    .insert(id, request.clone());
                self.emit(PageEvent::Request(request.clone()));
                if p["isBlocked"] == Value::Bool(true) {
                    self.route_request(request);
                }
            }
            "network.responseStarted" => {
                let request = self.request_of(p);
                if request.existing_response().is_none() {
                    let response = request.responded(p);
                    self.emit(PageEvent::Response(response));
                }
            }
            "network.responseCompleted" => {
                let request = self.request_of(p);
                if request.existing_response().is_none() {
                    let response = request.responded(p);
                    self.emit(PageEvent::Response(response));
                }
                request.completed(p);
                let redirect = (300..400).contains(
                    &p.pointer("/response/status")
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                ) && p
                    .pointer("/response/headers")
                    .and_then(Value::as_array)
                    .is_some_and(|hs| {
                        hs.iter().any(|h| {
                            h["name"]
                                .as_str()
                                .is_some_and(|n| n.eq_ignore_ascii_case("location"))
                        })
                    });
                if !redirect {
                    self.inner
                        .requests
                        .lock()
                        .unwrap()
                        .remove(&request.inner.id);
                }
                self.emit(PageEvent::RequestFinished(request));
            }
            "network.fetchError" => {
                let request = self.request_of(p);
                request.failed(&text("errorText"));
                self.inner
                    .requests
                    .lock()
                    .unwrap()
                    .remove(&request.inner.id);
                self.emit(PageEvent::RequestFailed(request));
            }
            "input.fileDialogOpened" => {
                let element = crate::application::firefox_handle::ElementHandle::from_remote(
                    self.clone(),
                    p["context"]
                        .as_str()
                        .unwrap_or(&self.inner.context)
                        .to_owned(),
                    p["element"].clone(),
                );
                self.emit(PageEvent::FileChooser(
                    crate::application::firefox_events::FileChooser {
                        page: self.clone(),
                        element,
                        multiple: p["multiple"] == Value::Bool(true),
                    },
                ));
            }
            "network.authRequired" if p["isBlocked"] == Value::Bool(true) => {
                self.answer_auth(p);
            }
            "browsingContext.downloadWillBegin" => {
                let download = Download::new(
                    text("url"),
                    text("suggestedFilename"),
                    self.inner.services.clone(),
                    Arc::downgrade(&self.inner),
                );
                self.inner
                    .downloads
                    .lock()
                    .unwrap()
                    .insert(text("navigation"), download.clone());
                self.emit(PageEvent::Download(download));
            }
            "browsingContext.downloadEnd" => {
                let download = self
                    .inner
                    .downloads
                    .lock()
                    .unwrap()
                    .remove(&text("navigation"));
                if let Some(download) = download {
                    match (p["status"].as_str(), p["filepath"].as_str()) {
                        (Some("complete"), Some(path)) => download.finish(Ok(PathBuf::from(path))),
                        (Some("complete"), None) => {
                            download.finish(Err("download was not saved".into()))
                        }
                        _ => download.finish(Err("canceled".into())),
                    }
                }
            }
            "browsingContext.domContentLoaded" => {
                self.emit(PageEvent::DomContentLoaded);
            }
            "script.realmCreated" if p["type"] == "window" => {
                self.inner.realms.lock().unwrap().insert(text("realm"));
            }
            "script.message" if p["channel"] == WS_CHANNEL => {
                let realm = p
                    .pointer("/source/realm")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(msg) = p
                    .pointer("/data/value")
                    .and_then(Value::as_str)
                    .and_then(|m| serde_json::from_str::<Value>(m).ok())
                {
                    self.websocket_message(realm, &msg);
                }
            }
            "browsingContext.load" => {
                self.emit(PageEvent::Load);
            }
            "browsingContext.navigationStarted" => {
                if let (Some(ctx), Some(origin)) = (
                    self.inner.owner.upgrade(),
                    crate::domain::firefox::origin_of(p["url"].as_str().unwrap_or_default()),
                ) {
                    let names = ctx.granted_any.lock().unwrap().clone();
                    if !names.is_empty() {
                        let context = BrowserContext::from_inner(ctx);
                        tokio::spawn(async move {
                            let _ = context
                                .set_permissions_for(&origin, &names, "granted")
                                .await;
                        });
                    }
                }
            }
            "browsingContext.contextDestroyed" => {
                self.emit(PageEvent::Close);
            }
            _ => {}
        }
    }

    /// An event about another browsing context or realm that may concern
    /// this page: a pop-up it opened, its dedicated workers.
    fn foreign_event(&self, method: &str, p: &Value) {
        match method {
            "browsingContext.contextCreated"
                if p["originalOpener"].as_str() == Some(self.inner.context.as_str())
                    && p["parent"].is_null() =>
            {
                let Some(owner) = self.inner.owner.upgrade() else {
                    return;
                };
                let id = p["context"].as_str().unwrap_or_default().to_owned();
                let url = p["url"].as_str().unwrap_or("about:blank").to_owned();
                // Subscribed now, before the pop-up's next events arrive.
                let events = self.inner.bidi.subscribe();
                let opener = self.clone();
                tokio::spawn(async move {
                    if let Ok(popup) = attach_page(&owner, id, url, events).await {
                        *popup.inner.opener.lock().unwrap() = Some(opener.clone());
                        popup.refresh_url().await;
                        opener.emit(PageEvent::Popup(popup));
                    }
                });
            }
            "script.realmCreated" if p["type"] == "dedicated-worker" => {
                let owned = p["owners"].as_array().is_some_and(|owners| {
                    let realms = self.inner.realms.lock().unwrap();
                    owners
                        .iter()
                        .any(|o| o.as_str().is_some_and(|o| realms.contains(o)))
                });
                if owned {
                    let worker = Worker::new(
                        self.clone(),
                        p["realm"].as_str().unwrap_or_default().to_owned(),
                        p["origin"].as_str().unwrap_or_default().to_owned(),
                    );
                    self.inner.workers.lock().unwrap().push(worker.clone());
                    self.emit(PageEvent::Worker(worker));
                }
            }
            "script.realmDestroyed" => {
                let realm = p["realm"].as_str().unwrap_or_default();
                self.inner.realms.lock().unwrap().remove(realm);
                let mut workers = self.inner.workers.lock().unwrap();
                if let Some(i) = workers.iter().position(|w| w.realm() == realm) {
                    workers.remove(i).mark_closed();
                }
            }
            _ => {}
        }
    }

    /// Re-reads the URL (events before the page was attached are missed).
    async fn refresh_url(&self) {
        if let Ok(tree) = self
            .bidi_call(
                "browsingContext.getTree",
                json!({"root": self.inner.context, "maxDepth": 0}),
            )
            .await
            && let Some(url) = tree.pointer("/contexts/0/url").and_then(Value::as_str)
        {
            *self.inner.url.lock().unwrap() = url.to_owned();
        }
    }

    /// One report of the WebSocket preload script.
    fn websocket_message(&self, realm: &str, m: &Value) {
        let key = format!("{realm}:{}", m["id"]);
        let payload = || match (m["text"].as_str(), m["b64"].as_str()) {
            (_, Some(b64)) => WsPayload::Binary(
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default(),
            ),
            (text, None) => WsPayload::Text(text.unwrap_or_default().to_owned()),
        };
        match m["type"].as_str() {
            Some("open") => {
                let ws = WebSocket::new(m["url"].as_str().unwrap_or_default().to_owned());
                self.inner
                    .websockets
                    .lock()
                    .unwrap()
                    .insert(key, ws.clone());
                self.emit(PageEvent::WebSocket(ws));
            }
            Some(kind) => {
                let ws = {
                    let mut sockets = self.inner.websockets.lock().unwrap();
                    if kind == "close" {
                        sockets.remove(&key)
                    } else {
                        sockets.get(&key).cloned()
                    }
                };
                let Some(ws) = ws else { return };
                match kind {
                    "sent" => ws.push(WebSocketEvent::FrameSent(payload())),
                    "recv" => ws.push(WebSocketEvent::FrameReceived(payload())),
                    "error" => ws.push(WebSocketEvent::Error(
                        m["message"].as_str().unwrap_or_default().to_owned(),
                    )),
                    "close" => ws.push(WebSocketEvent::Close),
                    _ => {}
                }
            }
            None => {}
        }
    }

    /// `page.workers()`: the page's dedicated workers.
    pub fn workers(&self) -> Vec<Worker> {
        self.inner.workers.lock().unwrap().clone()
    }

    /// Any BiDi command (for the engine's other modules).
    pub(crate) async fn bidi_call(&self, method: &str, params: Value) -> Result<Value> {
        self.check_open()?;
        self.inner.bidi.call(method, params).await
    }

    /// Hands a blocked request to the newest matching route handler, or
    /// lets it through.
    fn route_request(&self, request: Request) {
        let id = request.inner.id.clone();
        let find = |routes: &Mutex<Vec<RouteEntry>>| {
            routes
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|e| e.matcher.matches(request.url()))
                .map(|e| e.handler.clone())
        };
        let handler = find(&self.inner.routes).or_else(|| find(&self.inner.context_routes));
        let page = self.clone();
        tokio::spawn(async move {
            match handler {
                Some(handler) => {
                    handler(Route {
                        page,
                        id,
                        request,
                        handled: Arc::default(),
                    })
                    .await
                }
                None => {
                    let _ = page
                        .bidi_call("network.continueRequest", json!({"request": id}))
                        .await;
                }
            }
        });
    }

    /// Registers a page route.
    pub(crate) async fn add_route(&self, entry: RouteEntry) -> Result<()> {
        self.check_open()?;
        self.inner.routes.lock().unwrap().push(entry);
        self.sync_intercept().await
    }

    /// Removes page routes.
    pub(crate) async fn remove_routes(&self, remove: impl Fn(&RouteEntry) -> bool) -> Result<()> {
        self.inner.routes.lock().unwrap().retain(|e| !remove(e));
        self.sync_intercept().await
    }

    /// The page has a `beforeRequestSent` network intercept exactly while
    /// it or its context has routes.
    pub(crate) async fn sync_intercept(&self) -> Result<()> {
        let wanted = !self.inner.routes.lock().unwrap().is_empty()
            || !self.inner.context_routes.lock().unwrap().is_empty();
        let mut intercept = self.inner.intercept.lock().await;
        match (wanted, intercept.is_some()) {
            (true, false) => {
                let r = self
                    .bidi_call(
                        "network.addIntercept",
                        json!({"phases": ["beforeRequestSent"], "contexts": [self.inner.context]}),
                    )
                    .await?;
                *intercept = r
                    .get("intercept")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            (false, true) => {
                if let Some(id) = intercept.take() {
                    self.bidi_call("network.removeIntercept", json!({"intercept": id}))
                        .await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Answers a `network.authRequired` with the context's
    /// `httpCredentials` (once per request, for their origin if set), or
    /// cancels it.
    fn answer_auth(&self, p: &Value) {
        let id = p
            .pointer("/request/request")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let url = p
            .pointer("/request/url")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let first = self.inner.auth_attempts.lock().unwrap().insert(id.clone());
        let credentials = self
            .inner
            .owner
            .upgrade()
            .and_then(|ctx| ctx.credentials.lock().unwrap().clone())
            .filter(|_| first)
            .filter(|c| match &c.origin {
                None => true,
                Some(origin) => url::Url::parse(url)
                    .map(|u| {
                        u.origin()
                            .ascii_serialization()
                            .eq_ignore_ascii_case(origin)
                    })
                    .unwrap_or(false),
            });
        let params = match credentials {
            Some(c) => json!({"request": id, "action": "provideCredentials",
                "credentials": {"type": "password", "username": c.username, "password": c.password}}),
            None => json!({"request": id, "action": "cancel"}),
        };
        let page = self.clone();
        tokio::spawn(async move {
            let _ = page.bidi_call("network.continueWithAuth", params).await;
        });
    }

    /// Accepts or dismisses the open dialog.
    pub(crate) async fn handle_user_prompt(&self, accept: bool, text: Option<&str>) -> Result<()> {
        self.inner
            .bidi
            .call(
                "browsingContext.handleUserPrompt",
                prompt_params(&self.inner.context, accept, text),
            )
            .await?;
        Ok(())
    }

    /// `page.addInitScript(source)`: runs before any page script in every
    /// later document of this page.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "script.addPreloadScript",
                json!({"functionDeclaration": init_script_function(source),
                       "contexts": [self.inner.context]}),
            )
            .await?;
        Ok(())
    }

    /// `page.frames()`: the main frame and every iframe, depth first.
    pub async fn frames(&self) -> Result<Vec<Frame>> {
        let tree = self
            .bidi_call(
                "browsingContext.getTree",
                json!({"root": self.inner.context}),
            )
            .await?;
        let mut out = Vec::new();
        fn walk(page: &Page, node: &Value, parent: Option<&Frame>, out: &mut Vec<Frame>) {
            let frame = Frame {
                page: page.clone(),
                context: node["context"].as_str().unwrap_or_default().to_owned(),
                url: node["url"].as_str().unwrap_or_default().to_owned(),
                parent: parent.map(|p| Box::new(p.clone())),
            };
            out.push(frame.clone());
            for child in node["children"].as_array().into_iter().flatten() {
                walk(page, child, Some(&frame), out);
            }
        }
        for root in tree["contexts"].as_array().into_iter().flatten() {
            walk(self, root, None, &mut out);
        }
        Ok(out)
    }

    /// `page.video()`: the recording, when the context records video.
    pub fn video(&self) -> Option<Video> {
        self.inner.video.lock().unwrap().clone()
    }

    /// The trace and HAR id of this page (`page@<context>`).
    pub(crate) fn page_id(&self) -> String {
        format!("page@{}", self.inner.context.to_lowercase())
    }

    pub(crate) fn services(&self) -> &Services {
        &self.inner.services
    }

    pub(crate) fn options(&self) -> &ContextOptions {
        &self.inner.options
    }

    /// Runs an API call, recorded in the context's trace when tracing.
    pub(crate) async fn traced<T>(
        &self,
        class: &str,
        method: &str,
        params: Value,
        f: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let recorders = self.inner.recorders.clone();
        recorders.traced(self, class, method, params, f).await
    }

    /// `page.keyboard()`.
    pub fn keyboard(&self) -> Keyboard {
        Keyboard::new(self.clone(), self.inner.input.clone())
    }

    /// `page.mouse()`.
    pub fn mouse(&self) -> Mouse {
        Mouse::new(self.clone(), self.inner.input.clone())
    }

    /// The node `selector` matches in `context` as a BiDi shared reference
    /// (`None` when nothing matches; strict).
    pub(crate) async fn shared_node_in(
        &self,
        context: &str,
        selector: &str,
    ) -> Result<Option<String>> {
        // Installs the injected script in that frame first.
        self.injected_call_in(context, "() => 0", &[]).await?;
        let expression = format!(
            "(() => {{ const i = globalThis.{INJECTED}; const parsed = i.parseSelector({}); \
             const els = i.querySelectorAll(parsed, document); \
             if (els.length > 1) throw i.strictModeViolationError(parsed, els); return els[0] || null; }})()",
            Value::String(selector.to_owned())
        );
        let r = self
            .script_evaluate_in(context, &expression, Some(UTILITY))
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        Ok(r.pointer("/result/sharedId")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// `page.url()`: the main frame's URL.
    pub fn url(&self) -> String {
        self.inner.url.lock().unwrap().clone()
    }

    /// `page.content()`: the doctype and `documentElement.outerHTML`.
    pub async fn content(&self) -> Result<String> {
        let context = self.inner.context.clone();
        self.content_in(&context).await
    }

    /// `frame.content()` of a browsing context.
    pub(crate) async fn content_in(&self, context: &str) -> Result<String> {
        let v = self
            .call_json_in(
                context,
                "() => { let r = ''; if (document.doctype) r = new XMLSerializer().serializeToString(document.doctype); \
                 if (document.documentElement) r += document.documentElement.outerHTML; return r; }",
                &[],
                Some(UTILITY),
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `page.reload()`: reloads and waits for `load`.
    pub async fn reload(&self) -> Result<()> {
        self.traced(
            "Page",
            "reload",
            serde_json::json!({"waitUntil": "load"}),
            self.reload_untraced(),
        )
        .await
    }

    async fn reload_untraced(&self) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "browsingContext.reload",
                json!({"context": self.inner.context, "wait": "complete"}),
            )
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        Ok(())
    }

    async fn traverse(&self, delta: i64) -> Result<()> {
        self.check_open()?;
        match self
            .inner
            .bidi
            .call(
                "browsingContext.traverseHistory",
                json!({"context": self.inner.context, "delta": delta}),
            )
            .await
        {
            Ok(_) => Ok(()),
            // Playwright resolves with null when there is nowhere to go.
            Err(Error::Protocol { message, .. })
                if message.starts_with("no such history entry") =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// `page.goBack()` (nothing happens when there is no previous entry).
    pub async fn go_back(&self) -> Result<()> {
        self.traced(
            "Page",
            "goBack",
            serde_json::json!({"waitUntil": "load"}),
            self.go_back_untraced(),
        )
        .await
    }

    async fn go_back_untraced(&self) -> Result<()> {
        self.traverse(-1).await
    }

    /// `page.goForward()` (nothing happens when there is no next entry).
    pub async fn go_forward(&self) -> Result<()> {
        self.traced(
            "Page",
            "goForward",
            serde_json::json!({"waitUntil": "load"}),
            self.go_forward_untraced(),
        )
        .await
    }

    async fn go_forward_untraced(&self) -> Result<()> {
        self.traverse(1).await
    }

    /// `page.waitForTimeout(ms)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.sleep(duration).await
    }

    /// `page.waitForFunction(fn | expression)`: polls until it returns a
    /// truthy value (Playwright's 30 s default timeout) and resolves with it.
    pub async fn wait_for_function(&self, source: &str) -> Result<Value> {
        let context = self.inner.context.clone();
        self.wait_for_function_in(&context, source).await
    }

    /// `frame.waitForFunction(...)` in a browsing context.
    pub(crate) async fn wait_for_function_in(&self, context: &str, source: &str) -> Result<Value> {
        let timeout = self.timeout();
        let deadline = self.now() + timeout;
        loop {
            match self
                .evaluate_value_in(json!({"context": context}), source, None)
                .await
            {
                Ok(v) if truthy(&v) => return Ok(v),
                Ok(_) => {}
                // A navigation can destroy the realm mid-poll; try again.
                Err(Error::Protocol { .. }) => {}
                Err(e) => return Err(e),
            }
            if self.now() >= deadline {
                return Err(Error::Timeout {
                    ms: timeout.as_millis() as u64,
                    action: "waiting for function".into(),
                });
            }
            self.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `page.title()`.
    pub async fn title(&self) -> Result<String> {
        self.evaluate("document.title").await
    }

    /// `page.evaluate(expressionOrFunction)`.
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

    /// Playwright's string semantics: an expression is returned, a function
    /// is called with the argument; values come back as JSON. Sources that
    /// are statements rather than one expression run as a script.
    async fn evaluate_value(&self, source: &str, arg: Option<&Value>) -> Result<Value> {
        let target = json!({"context": self.inner.context});
        self.evaluate_value_in(target, source, arg).await
    }

    /// [`Page::evaluate_value`] in a BiDi script target (a frame or a
    /// worker realm).
    pub(crate) async fn evaluate_value_in(
        &self,
        target: Value,
        source: &str,
        arg: Option<&Value>,
    ) -> Result<Value> {
        self.check_open()?;
        let source = source.trim().trim_end_matches(';');
        let arg = arg
            .map(Value::to_string)
            .unwrap_or_else(|| "undefined".into());
        let wrapped = format!(
            "(async (__chromiumoxide_arg) => {{ const __v = (\n{source}\n); \
             const __r = typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; \
             return __r === undefined ? undefined : JSON.stringify(__r); }})({arg})"
        );
        let r = self
            .script_evaluate_target(target.clone(), &wrapped)
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            let message = exception_text(&r);
            if !message.starts_with("SyntaxError") {
                return Err(Error::Evaluation(message));
            }
            let r = self.script_evaluate_target(target, source).await?;
            if r.get("type").and_then(Value::as_str) == Some("exception") {
                return Err(Error::Evaluation(exception_text(&r)));
            }
            return Ok(remote_value_to_json(&r["result"]));
        }
        match r.pointer("/result/value").and_then(Value::as_str) {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    /// Calls a function source with JSON arguments in the page (`None`) or
    /// in a sandbox; returns its JSON result.
    async fn call_json(
        &self,
        function: &str,
        args: &[Value],
        sandbox: Option<&str>,
    ) -> Result<Value> {
        let context = self.inner.context.clone();
        self.call_json_in(&context, function, args, sandbox).await
    }

    /// [`Page::call_json`] in a given browsing context (a frame).
    pub(crate) async fn call_json_in(
        &self,
        context: &str,
        function: &str,
        args: &[Value],
        sandbox: Option<&str>,
    ) -> Result<Value> {
        self.check_open()?;
        let args: Vec<String> = args.iter().map(Value::to_string).collect();
        let expression = format!(
            "(async () => {{ const __r = await ({function})({}); \
             return __r === undefined ? undefined : JSON.stringify(__r); }})()",
            args.join(", ")
        );
        let r = self
            .script_evaluate_in(context, &expression, sandbox)
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        match r.pointer("/result/value").and_then(Value::as_str) {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    pub(crate) async fn script_evaluate_in(
        &self,
        context: &str,
        expression: &str,
        sandbox: Option<&str>,
    ) -> Result<Value> {
        let mut target = json!({"context": context});
        if let Some(sandbox) = sandbox {
            target["sandbox"] = json!(sandbox);
        }
        self.script_evaluate_target(target, expression).await
    }

    pub(crate) async fn script_evaluate_target(
        &self,
        target: Value,
        expression: &str,
    ) -> Result<Value> {
        self.inner
            .bidi
            .call(
                "script.evaluate",
                json!({
                    "expression": expression,
                    "target": target,
                    "awaitPromise": true,
                    "resultOwnership": "none",
                    "userActivation": true,
                }),
            )
            .await
    }

    /// `input.performActions` on this page.
    pub(crate) async fn perform_actions(&self, actions: Value) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "input.performActions",
                json!({"context": self.inner.context, "actions": actions}),
            )
            .await?;
        Ok(())
    }

    /// Inserts text at the caret like typing it without key events
    /// (Playwright's `keyboard.insertText`; BiDi has no command for it).
    pub(crate) async fn insert_text_in(&self, context: &str, text: &str) -> Result<()> {
        self.call_json_in(
            context,
            "(text) => { document.execCommand('insertText', false, text); }",
            &[json!(text)],
            None,
        )
        .await?;
        Ok(())
    }

    /// The main frame's browsing context id.
    pub(crate) fn main_context(&self) -> String {
        self.inner.context.clone()
    }

    /// The browsing context of the iframe `selector` matches inside
    /// `context`, and the offset of its content box in that frame's
    /// viewport; `None` while there is no such iframe.
    pub(crate) async fn child_frame(
        &self,
        context: &str,
        selector: &str,
    ) -> Result<Option<(String, (f64, f64))>> {
        let offset = self
            .injected_call_in(
                context,
                "(i, selector) => { const parsed = i.parseSelector(selector); \
                 const els = i.querySelectorAll(parsed, document); \
                 if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                 const f = els[0]; if (!f || !f.contentWindow) return null; \
                 const r = f.getBoundingClientRect(); const s = getComputedStyle(f); \
                 return { x: r.left + f.clientLeft + parseFloat(s.paddingLeft), \
                          y: r.top + f.clientTop + parseFloat(s.paddingTop) }; }",
                &[json!(selector)],
            )
            .await?;
        if offset.is_null() {
            return Ok(None);
        }
        let expression = format!(
            "(() => {{ const i = globalThis.{INJECTED}; const parsed = i.parseSelector({}); \
             const f = i.querySelectorAll(parsed, document)[0]; return f ? f.contentWindow : null; }})()",
            Value::String(selector.to_owned())
        );
        let r = self
            .script_evaluate_in(context, &expression, Some(UTILITY))
            .await?;
        let child = r
            .pointer("/result/value/context")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(child.map(|c| {
            (
                c,
                (
                    offset["x"].as_f64().unwrap_or_default(),
                    offset["y"].as_f64().unwrap_or_default(),
                ),
            )
        }))
    }

    pub(crate) fn now(&self) -> Duration {
        Duration::from_millis(self.inner.services.clock.now_ms())
    }

    pub(crate) async fn sleep(&self, duration: Duration) {
        self.inner.services.clock.sleep(duration).await
    }

    /// Calls `body(injected, ...args)` in the utility sandbox of a browsing
    /// context (a frame), installing Playwright's injected script first when
    /// the document has none yet.
    pub(crate) async fn injected_call_in(
        &self,
        context: &str,
        body: &str,
        args: &[Value],
    ) -> Result<Value> {
        let call = format!(
            "(...args) => {{ const i = globalThis.{INJECTED}; \
             if (!i) return {{ __chromiumoxide_missing: true }}; return ({body})(i, ...args); }}"
        );
        let v = self
            .call_json_in(context, &call, args, Some(UTILITY))
            .await?;
        if v.get("__chromiumoxide_missing") != Some(&Value::Bool(true)) {
            return Ok(v);
        }
        let options = json!({
            "isUnderTest": false, "sdkLanguage": "javascript", "frameSeq": 0,
            "testIdAttributeName": "data-testid", "stableRafCount": 1, "browserName": "firefox",
            "isUtilityWorld": true,
            "customEngines": [{"name": "chromiumoxide-handle",
                               "source": crate::application::locator::HANDLE_ENGINE}],
        });
        let install = format!(
            "(() => {{ if (globalThis.{INJECTED}) return; const module = {{}};\n{INJECTED_SOURCE}\n\
             globalThis.{INJECTED} = new (module.exports.InjectedScript())(globalThis, {options}); \
             globalThis.{INJECTED}.__chromiumoxide_render_aria_yaml = json => renderAriaSnapshotAsYaml(json); }})()"
        );
        let r = self
            .script_evaluate_in(context, &install, Some(UTILITY))
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        self.call_json_in(context, &call, args, Some(UTILITY)).await
    }

    /// `page.setContent(html)`: replaces the document and waits for `load`
    /// (Playwright's `document.open/write/close`).
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.traced(
            "Frame",
            "setContent",
            serde_json::json!({"html": html, "waitUntil": "load"}),
            self.set_content_untraced(html),
        )
        .await
    }

    async fn set_content_untraced(&self, html: &str) -> Result<()> {
        let context = self.inner.context.clone();
        self.set_content_in(&context, html).await
    }

    /// `frame.setContent(html)` in a browsing context.
    pub(crate) async fn set_content_in(&self, context: &str, html: &str) -> Result<()> {
        self.call_json_in(
            context,
            "(html) => new Promise(resolve => { document.open(); \
             addEventListener('load', () => resolve(), { once: true }); \
             document.write(html); document.close(); \
             if (document.readyState === 'complete') resolve(); })",
            &[json!(html)],
            None,
        )
        .await?;
        Ok(())
    }

    /// `page.addStyleTag({ content | url | path })`.
    pub async fn add_style_tag(&self, tag: StyleTag) -> Result<()> {
        let context = self.inner.context.clone();
        self.add_style_tag_in(&context, tag).await
    }

    /// `frame.addStyleTag(...)` in a browsing context.
    pub(crate) async fn add_style_tag_in(&self, context: &str, tag: StyleTag) -> Result<()> {
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
        self.call_json_in(context, ADD_STYLE, &[content, url], None)
            .await?;
        Ok(())
    }

    /// `page.setViewportSize(size)` (the device scale factor stays).
    pub async fn set_viewport_size(&self, viewport: Viewport) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "browsingContext.setViewport",
                json!({"context": self.inner.context,
                       "viewport": {"width": viewport.width, "height": viewport.height}}),
            )
            .await?;
        *self.inner.viewport.lock().unwrap() = Some(viewport);
        Ok(())
    }

    /// `page.viewportSize()`.
    pub fn viewport_size(&self) -> Option<Viewport> {
        self.inner
            .viewport
            .lock()
            .unwrap()
            .or(self.inner.options.viewport)
    }

    /// `page.screenshot()`; chain options, then `.await` the image bytes.
    pub fn screenshot(&self) -> Screenshot<'_> {
        Screenshot {
            page: self,
            options: ScreenshotOptions::default(),
            path: None,
        }
    }

    /// Playwright's `Screenshotter.screenshotPage` with Firefox's
    /// `takeScreenshot`: always a document-relative rectangle.
    async fn take_screenshot(&self, o: &ScreenshotOptions) -> Result<Vec<u8>> {
        let quality = validate(o).map_err(|e| Error::InvalidArgument(e.0))?;
        if o.omit_background {
            return Err(Error::InvalidArgument(
                "omitBackground is not supported on Firefox over WebDriver BiDi".into(),
            ));
        }
        if o.scale == ScreenshotScale::Css
            && self.inner.options.device_scale_factor.unwrap_or(1.0) != 1.0
        {
            return Err(Error::InvalidArgument(
                "scale: \"css\" is not supported on Firefox over WebDriver BiDi".into(),
            ));
        }
        let format = match o.format {
            ImageFormat::Png => json!({"type": "image/png"}),
            ImageFormat::Jpeg => {
                json!({"type": "image/jpeg", "quality": f64::from(quality.unwrap_or(80)) / 100.0})
            }
            ImageFormat::Webp => {
                return Err(Error::InvalidArgument(
                    "webp screenshots are not supported on Firefox over WebDriver BiDi".into(),
                ));
            }
        };
        let viewport = match self.viewport_size() {
            Some(v) => Size {
                width: f64::from(v.width),
                height: f64::from(v.height),
            },
            None => {
                let v = self
                    .call_json(
                        "() => ({ width: innerWidth, height: innerHeight })",
                        &[],
                        Some(UTILITY),
                    )
                    .await?;
                Size {
                    width: v["width"].as_f64().unwrap_or_default(),
                    height: v["height"].as_f64().unwrap_or_default(),
                }
            }
        };
        let prepare = format!("({PREPARE})");
        self.call_json(
            &prepare,
            &[
                o.style.clone().map(Value::String).unwrap_or(Value::Null),
                json!(o.caret != Caret::Initial),
                json!(o.animations == Animations::Disabled),
                json!(false),
            ],
            Some(UTILITY),
        )
        .await?;
        let _ = self
            .call_json(
                "() => document.fonts.ready.then(() => undefined)",
                &[],
                Some(UTILITY),
            )
            .await;
        let shot = self.capture(o, viewport, format).await;
        let _ = self
            .call_json(&format!("({CLEANUP})"), &[], Some(UTILITY))
            .await;
        shot
    }

    async fn capture(
        &self,
        o: &ScreenshotOptions,
        viewport: Size,
        format: Value,
    ) -> Result<Vec<u8>> {
        let full_page = if o.full_page {
            let v = self
                .call_json(&format!("({FULL_PAGE_SIZE})"), &[], Some(UTILITY))
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
        // Playwright asks Juggler for the document rectangle under the
        // viewport (viewport rect + scroll); BiDi clips a document-origin
        // capture to the document, which can be smaller than the viewport,
        // so viewport shots use the viewport origin instead.
        let (origin, rect) =
            match plan_page(o, viewport, full_page).map_err(|e| Error::InvalidArgument(e.0))? {
                Target::Document { rect, .. } => ("document", rect),
                Target::Viewport(r) => ("viewport", r),
            };
        let r = self
            .inner
            .bidi
            .call(
                "browsingContext.captureScreenshot",
                json!({
                    "context": self.inner.context,
                    "origin": origin,
                    "format": format,
                    "clip": {"type": "box", "x": rect.x, "y": rect.y, "width": rect.width, "height": rect.height},
                }),
            )
            .await?;
        base64::engine::general_purpose::STANDARD
            .decode(r.get("data").and_then(Value::as_str).unwrap_or_default())
            .map_err(|e| Error::Serde(format!("screenshot data: {e}")))
    }

    /// `page.close()`: closes the tab (and the user context `new_page` made).
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self
            .inner
            .bidi
            .call(
                "browsingContext.close",
                json!({"context": self.inner.context}),
            )
            .await;
        let owned = self.inner.owned_context.lock().unwrap().take();
        if let Some(context) = owned {
            context.close().await?;
        }
        Ok(())
    }
}

/// A frame of a Firefox page (`page.frames()`).
#[derive(Clone, Debug)]
pub struct Frame {
    pub(crate) page: Page,
    pub(crate) context: String,
    pub(crate) url: String,
    pub(crate) parent: Option<Box<Frame>>,
}

impl Frame {
    /// The frame's browsing context id.
    pub(crate) fn context_id(&self) -> &str {
        &self.context
    }

    /// The frame's URL when it was listed.
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A pending `page.screenshot()` on Firefox.
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
    pub fn format(mut self, format: ImageFormat) -> Self {
        self.options.format = format;
        self
    }
    pub fn quality(mut self, quality: u8) -> Self {
        self.options.quality = Some(quality);
        self
    }
    pub fn caret(mut self, caret: Caret) -> Self {
        self.options.caret = caret;
        self
    }
    pub fn animations(mut self, animations: Animations) -> Self {
        self.options.animations = animations;
        self
    }
    /// Stylesheet applied while capturing.
    pub fn style(mut self, css: &str) -> Self {
        self.options.style = Some(css.to_owned());
        self
    }
    /// Also write the image to this file.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
    /// All options at once.
    pub fn options(mut self, options: ScreenshotOptions) -> Self {
        self.options = options;
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

/// The message of a `script.evaluate` exception, like Chromium's
/// (`Error: boom`).
pub(crate) fn exception_text(r: &Value) -> String {
    let details = &r["exceptionDetails"];
    details
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| remote_value_to_json(&details["exception"]).to_string())
}
