//! Tauri apps (`chromiumoxide::Tauri`). Tauri has no browser of its own:
//! on Windows its windows are WebView2 (Chromium), reached over CDP, so
//! each window is a [`Page`] with the whole Chromium API; on Linux they are
//! WebKitGTK, reached through `WebKitWebDriver` (WebDriver). macOS has no
//! automation interface for WKWebView.
//!
//! The app opts in: on Windows it passes `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
//! on to its WebView2 arguments (Tauri sets its own, which WebView2 then
//! prefers); on Linux Tauri reads `TAURI_WEBVIEW_AUTOMATION=true`, which
//! chromiumoxide sets. See `tests/fixtures/tauri-app`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::browser::Browser;
use crate::application::context::{BrowserContext, ContextEvent};
use crate::application::page::Page;
use crate::application::services::Services;
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};
use crate::domain::http::HttpRequest;
use crate::ports::{TauriCommand, TauriConnection};

/// Options for [`crate::Tauri::launch`].
#[derive(Debug, Clone)]
pub struct TauriLaunchOptions {
    /// The app's executable.
    pub executable_path: PathBuf,
    pub args: Vec<String>,
    /// Extra environment variables for the app.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the app's web view (default 60 s).
    pub timeout: Duration,
}

impl Default for TauriLaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: PathBuf::new(),
            args: Vec::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(60),
        }
    }
}

/// A running Tauri app. Cheap to clone.
#[derive(Clone)]
pub struct TauriApp {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Backend,
    services: Services,
    pid: Option<u32>,
}

enum Backend {
    /// Windows: WebView2 over CDP.
    Cdp {
        browser: Browser,
        context: BrowserContext,
    },
    /// Linux: a WebKitWebDriver session; `process` is the driver.
    WebDriver {
        window: WebDriverWindow,
        process: std::sync::Mutex<Option<Box<dyn crate::ports::BrowserProcess>>>,
    },
}

impl std::fmt::Debug for TauriApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TauriApp")
            .field("pid", &self.inner.pid)
            .finish()
    }
}

impl TauriApp {
    /// Starts the app and connects to its web view.
    pub async fn launch(services: Services, options: TauriLaunchOptions) -> Result<TauriApp> {
        let launched = services
            .launcher
            .launch_tauri(TauriCommand {
                program: options.executable_path.clone(),
                args: options.args.clone(),
                env: options.env.clone(),
                timeout_ms: options.timeout.as_millis() as u64,
            })
            .await?;
        let pid = launched.process.pid();
        let backend = match launched.connection {
            TauriConnection::Cdp(cdp) => {
                let (browser, context) = Browser::with_default_context(
                    services.clone(),
                    cdp,
                    Some(launched.process),
                    ContextOptions {
                        viewport: None,
                        ..Default::default()
                    },
                )
                .await?;
                Backend::Cdp { browser, context }
            }
            TauriConnection::WebDriver { endpoint, session } => Backend::WebDriver {
                window: WebDriverWindow::new(
                    services.clone(),
                    format!("{endpoint}/session/{session}"),
                ),
                process: std::sync::Mutex::new(Some(launched.process)),
            },
        };
        Ok(TauriApp {
            inner: Arc::new(Inner {
                backend,
                services,
                pid,
            }),
        })
    }

    /// The app's process id.
    pub fn pid(&self) -> Option<u32> {
        self.inner.pid
    }

    /// The context of the app's windows (Windows: WebView2 only).
    pub fn context(&self) -> Option<BrowserContext> {
        match &self.inner.backend {
            Backend::Cdp { context, .. } => Some(context.clone()),
            Backend::WebDriver { .. } => None,
        }
    }

    /// The open windows.
    pub fn windows(&self) -> Vec<TauriWindow> {
        match &self.inner.backend {
            Backend::Cdp { context, .. } => context
                .pages()
                .into_iter()
                .filter(|p| !p.is_closed())
                .map(TauriWindow::WebView2)
                .collect(),
            Backend::WebDriver { window, .. } => vec![TauriWindow::WebKitGtk(window.clone())],
        }
    }

    /// The first window, once it is open (30 s at most).
    pub async fn first_window(&self) -> Result<TauriWindow> {
        let context = match &self.inner.backend {
            Backend::Cdp { context, .. } => context.clone(),
            Backend::WebDriver { window, .. } => return Ok(TauriWindow::WebKitGtk(window.clone())),
        };
        let mut events = context.events();
        if let Some(p) = self.windows().into_iter().next() {
            return Ok(p);
        }
        let wait = async {
            while let Some(e) = events.next().await {
                if let ContextEvent::Page(p) = e {
                    return Ok(TauriWindow::WebView2(p));
                }
            }
            Err(Error::TargetClosed("the Tauri app closed".into()))
        };
        crate::application::wait::with_timeout(
            &*self.inner.services.clock,
            Duration::from_secs(30),
            "waiting for the Tauri app's first window",
            wait,
        )
        .await
    }

    /// Ends the app: its web view first, then the process (killed when it
    /// has not gone away by itself).
    pub async fn close(&self) -> Result<()> {
        match &self.inner.backend {
            Backend::Cdp { browser, .. } => {
                let process = browser.take_process();
                let _ = browser.close().await;
                if let Some(process) = process {
                    process.shutdown().await;
                }
            }
            Backend::WebDriver { window, process } => {
                let _ = window.end_session().await;
                let process = process.lock().unwrap().take();
                if let Some(process) = process {
                    process.shutdown().await;
                }
            }
        }
        Ok(())
    }
}

/// A window of a Tauri app: a WebView2 [`Page`] (Windows, the whole
/// Chromium API through [`TauriWindow::page`]) or a WebKitGTK window driven
/// over WebDriver (Linux).
#[derive(Clone, Debug)]
pub enum TauriWindow {
    WebView2(Page),
    WebKitGtk(WebDriverWindow),
}

/// The JSON of a page-side value: an expression's, or a function's result.
fn evaluate_script(source: &str) -> String {
    format!(
        "return (async () => {{ const __v = (\n{}\n); \
         const __r = typeof __v === 'function' ? await __v() : await __v; \
         return __r === undefined ? null : JSON.stringify(__r); }})()",
        source.trim().trim_end_matches(';')
    )
}

impl TauriWindow {
    /// The Chromium page (Windows).
    pub fn page(&self) -> Option<&Page> {
        match self {
            TauriWindow::WebView2(p) => Some(p),
            TauriWindow::WebKitGtk(_) => None,
        }
    }

    /// `page.goto(url)`.
    pub async fn goto(&self, url: &str) -> Result<()> {
        match self {
            TauriWindow::WebView2(p) => p.goto(url).await.map(drop),
            TauriWindow::WebKitGtk(w) => w.goto(url).await,
        }
    }

    /// `page.title()`.
    pub async fn title(&self) -> Result<String> {
        match self {
            TauriWindow::WebView2(p) => p.title().await,
            TauriWindow::WebKitGtk(w) => w.title().await,
        }
    }

    /// `page.evaluate(expression or function)` with a JSON result.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        match self {
            TauriWindow::WebView2(p) => p.evaluate(source).await,
            TauriWindow::WebKitGtk(w) => w.evaluate(source).await,
        }
    }

    /// `page.waitForFunction(source)`: until it is truthy (30 s at most).
    pub async fn wait_for_function(&self, source: &str) -> Result<()> {
        match self {
            TauriWindow::WebView2(p) => p.wait_for_function(source).await.map(drop),
            TauriWindow::WebKitGtk(w) => w.wait_for_function(source).await,
        }
    }

    /// A PNG of the window's viewport.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        match self {
            TauriWindow::WebView2(p) => p.screenshot().await,
            TauriWindow::WebKitGtk(w) => w.screenshot().await,
        }
    }

    /// An element by selector: any Playwright selector on Windows, a CSS
    /// selector on Linux (WebDriver's locator strategy).
    pub fn locator(&self, selector: &str) -> TauriLocator {
        TauriLocator {
            window: self.clone(),
            selector: selector.to_owned(),
        }
    }
}

/// An element of a [`TauriWindow`].
#[derive(Clone, Debug)]
pub struct TauriLocator {
    window: TauriWindow,
    selector: String,
}

impl TauriLocator {
    /// `locator.click()`.
    pub async fn click(&self) -> Result<()> {
        match &self.window {
            TauriWindow::WebView2(p) => p.locator(&self.selector).click().await,
            TauriWindow::WebKitGtk(w) => w.locator(&self.selector).click().await,
        }
    }

    /// `locator.innerText()`.
    pub async fn inner_text(&self) -> Result<String> {
        match &self.window {
            TauriWindow::WebView2(p) => p.locator(&self.selector).inner_text().await,
            TauriWindow::WebKitGtk(w) => w.locator(&self.selector).inner_text().await,
        }
    }
}

/// An element of a [`WebDriverWindow`], found by CSS selector when used.
#[derive(Clone, Debug)]
pub struct WebDriverLocator {
    window: WebDriverWindow,
    selector: String,
}

impl WebDriverLocator {
    /// `locator.click()` (WebDriver's element click: scrolls into view,
    /// checks the element is not covered).
    pub async fn click(&self) -> Result<()> {
        let id = self.window.element(&self.selector).await?;
        self.window
            .command("POST", &format!("/element/{id}/click"), Some(json!({})))
            .await
            .map(drop)
    }

    /// `locator.innerText()` (WebDriver's rendered element text).
    pub async fn inner_text(&self) -> Result<String> {
        let id = self.window.element(&self.selector).await?;
        Ok(self
            .window
            .command("GET", &format!("/element/{id}/text"), None)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// `locator.fill(value)`: clears the field, then types `value`.
    pub async fn fill(&self, value: &str) -> Result<()> {
        let id = self.window.element(&self.selector).await?;
        self.window
            .command("POST", &format!("/element/{id}/clear"), Some(json!({})))
            .await?;
        self.window
            .command(
                "POST",
                &format!("/element/{id}/value"),
                Some(json!({"text": value})),
            )
            .await
            .map(drop)
    }
}

/// A window driven over a WebDriver session (a Tauri app's WebKitGTK
/// window, or Safari). Cheap to clone.
#[derive(Clone)]
pub struct WebDriverWindow {
    inner: Arc<WdInner>,
}

struct WdInner {
    services: Services,
    /// `http://127.0.0.1:<port>/session/<id>`.
    base: String,
}

impl std::fmt::Debug for WebDriverWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebDriverWindow")
            .field("session", &self.inner.base)
            .finish()
    }
}

impl WebDriverWindow {
    /// A window of the session at `base` (`http://host:port/session/<id>`).
    pub(crate) fn new(services: Services, base: String) -> Self {
        WebDriverWindow {
            inner: Arc::new(WdInner { services, base }),
        }
    }

    /// `page.goto(url)`: navigates and waits for the load (WebDriver's
    /// normal page load strategy).
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.command("POST", "/url", Some(json!({"url": url})))
            .await
            .map(drop)
    }

    /// `page.title()`.
    pub async fn title(&self) -> Result<String> {
        Ok(self
            .command("GET", "/title", None)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// `page.url()`.
    pub async fn url(&self) -> Result<String> {
        Ok(self
            .command("GET", "/url", None)
            .await?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// `page.evaluate(expression or function)` with a JSON result.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.evaluate_value(source).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `page.waitForFunction(source)`: until it is truthy (30 s at most).
    pub async fn wait_for_function(&self, source: &str) -> Result<()> {
        let clock = self.inner.services.clock.clone();
        let deadline = clock.now_ms() + 30_000;
        loop {
            if let Ok(v) = self.evaluate_value(&format!("!!({source})")).await
                && v == Value::Bool(true)
            {
                return Ok(());
            }
            if clock.now_ms() > deadline {
                return Err(Error::Timeout {
                    ms: 30_000,
                    action: format!("waiting for function: {source}"),
                });
            }
            clock.sleep(Duration::from_millis(50)).await;
        }
    }

    /// A PNG of the viewport.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        let data = self.command("GET", "/screenshot", None).await?;
        base64::engine::general_purpose::STANDARD
            .decode(data.as_str().unwrap_or_default())
            .map_err(|e| Error::Serde(format!("screenshot data: {e}")))
    }

    /// An element by CSS selector (WebDriver's locator strategy).
    pub fn locator(&self, selector: &str) -> WebDriverLocator {
        WebDriverLocator {
            window: self.clone(),
            selector: selector.to_owned(),
        }
    }

    /// Ends the WebDriver session (closes the browser window).
    pub(crate) async fn end_session(&self) -> Result<()> {
        self.command("DELETE", "", None).await.map(drop)
    }

    /// A WebDriver command; its `value`, or the WebDriver error.
    async fn command(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let body = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
        let response = self
            .inner
            .services
            .http
            .send(HttpRequest {
                method: method.to_owned(),
                url: format!("{}{path}", self.inner.base),
                headers: vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("Content-Length".into(), body.len().to_string()),
                ],
                body,
                timeout_ms: Some(60_000),
                ..Default::default()
            })
            .await
            .map_err(|e| Error::Request(format!("WebDriver {method} {path}: {e}")))?;
        let mut v: Value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        let value = v.get_mut("value").map(Value::take).unwrap_or(Value::Null);
        if response.status >= 400 {
            return Err(Error::Protocol {
                method: format!("WebDriver {method} {path}"),
                code: i64::from(response.status),
                message: format!(
                    "{}: {}",
                    value["error"].as_str().unwrap_or("error"),
                    value["message"].as_str().unwrap_or_default()
                ),
            });
        }
        Ok(value)
    }

    async fn evaluate_value(&self, source: &str) -> Result<Value> {
        let v = self
            .command(
                "POST",
                "/execute/sync",
                Some(json!({"script": evaluate_script(source), "args": []})),
            )
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Evaluation(message),
                other => other,
            })?;
        match v.as_str() {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    /// The WebDriver id of the element `selector` (CSS) finds.
    async fn element(&self, selector: &str) -> Result<String> {
        let v = self
            .command(
                "POST",
                "/element",
                Some(json!({"using": "css selector", "value": selector})),
            )
            .await?;
        v.as_object()
            .and_then(|o| o.values().next())
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Evaluation(format!("no element for {selector}")))
    }
}
