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
use crate::application::locator::INJECTED_SOURCE;
use crate::application::screenshot::{CLEANUP, FULL_PAGE_SIZE, PREPARE};
use crate::application::services::Services;
use crate::domain::emulation::{ContextOptions, Viewport};
use crate::domain::error::{Error, Result};
use crate::domain::firefox::{
    command_line, profile_files, remote_value_to_json, unsupported_context_options,
};
use crate::domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale, Size, Target,
    plan_page, validate,
};
use crate::ports::{BidiSession, BrowserProcess, FirefoxCommand};

/// The sandbox (BiDi's isolated world) for Playwright's utility scripts.
const UTILITY: &str = "__chromiumoxide_utility";

/// The global holding Playwright's injected script in the sandbox.
const INJECTED: &str = "__chromiumoxide_injected";

/// Options for [`crate::Firefox::launch`] (Playwright's `firefox.launch`).
#[derive(Debug, Clone)]
pub struct FirefoxLaunchOptions {
    /// The Firefox binary; `None` finds Playwright's newest Firefox
    /// (`PLAYWRIGHT_BROWSERS_PATH`, then `~/.cache/ms-playwright`).
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
    let mut dirs = Vec::new();
    if let Some(p) = services.env.var("PLAYWRIGHT_BROWSERS_PATH") {
        dirs.push(PathBuf::from(p));
    }
    if let Some(home) = services.env.home_dir() {
        dirs.push(home.join(".cache/ms-playwright"));
    }
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

/// A Firefox browser driven over WebDriver BiDi.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<BrowserInner>,
}

struct BrowserInner {
    bidi: Arc<dyn BidiSession>,
    services: Services,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
    profile: Option<PathBuf>,
    version: String,
    closed: AtomicBool,
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
        let profile = services
            .fs
            .create_temp_dir("chromiumoxide-firefox-profile-")
            .map_err(|e| Error::Launch(format!("cannot create the profile: {e}")))?;
        for (path, contents) in profile_files(options.headless) {
            let path = profile.join(path);
            let written = match path.parent() {
                Some(dir) => services.fs.create_dir_all(dir),
                None => Ok(()),
            }
            .and_then(|()| services.fs.write(&path, contents.as_bytes()));
            if let Err(e) = written {
                let _ = services.fs.remove_dir_all(&profile);
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
                let _ = services.fs.remove_dir_all(&profile);
                return Err(e);
            }
        };
        let session = launched
            .bidi
            .call(
                "session.new",
                json!({"capabilities": {"alwaysMatch": {"acceptInsecureCerts": false}}}),
            )
            .await;
        let session = match session {
            Ok(s) => s,
            Err(e) => {
                launched.process.shutdown().await;
                let _ = services.fs.remove_dir_all(&profile);
                return Err(e);
            }
        };
        if let Err(e) = launched
            .bidi
            .call("session.subscribe", json!({"events": ["browsingContext"]}))
            .await
        {
            launched.process.shutdown().await;
            let _ = services.fs.remove_dir_all(&profile);
            return Err(e);
        }
        let version = session
            .pointer("/capabilities/browserVersion")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Browser {
            inner: Arc::new(BrowserInner {
                bidi: launched.bidi,
                services,
                process: Mutex::new(Some(launched.process)),
                profile: Some(profile),
                version,
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
        let bidi = &self.inner.bidi;
        let mut create = json!({});
        if options.ignore_https_errors {
            create["acceptInsecureCerts"] = json!(true);
        }
        let user_context = bidi
            .call("browser.createUserContext", create)
            .await?
            .get("userContext")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let context = BrowserContext {
            inner: Arc::new(ContextInner {
                bidi: bidi.clone(),
                services: self.inner.services.clone(),
                user_context: user_context.clone(),
                options: Arc::new(options),
                closed: AtomicBool::new(false),
            }),
        };
        if let Err(e) = context.apply_emulation().await {
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
        let _ = self.inner.bidi.call("browser.close", json!({})).await;
        let process = self.inner.process.lock().unwrap().take();
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
    inner: Arc<ContextInner>,
}

struct ContextInner {
    bidi: Arc<dyn BidiSession>,
    services: Services,
    user_context: String,
    options: Arc<ContextOptions>,
    closed: AtomicBool,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::BrowserContext")
            .field("user_context", &self.inner.user_context)
            .finish()
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

    /// `context.newPage()`: a new window in this user context.
    pub async fn new_page(&self) -> Result<Page> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("context has been closed".into()));
        }
        let mut events = self.inner.bidi.subscribe();
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
        let inner = Arc::new(PageInner {
            bidi: self.inner.bidi.clone(),
            services: self.inner.services.clone(),
            context: context.clone(),
            options: self.inner.options.clone(),
            viewport: Mutex::new(None),
            owned_context: Mutex::new(None),
            url: Mutex::new("about:blank".to_owned()),
            closed: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            use futures::StreamExt;
            while let Some(event) = events.next().await {
                if event.params.get("context").and_then(Value::as_str) != Some(context.as_str()) {
                    continue;
                }
                let Some(page) = weak.upgrade() else {
                    break;
                };
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
                        break;
                    }
                    _ => {}
                }
            }
        });
        Ok(Page { inner })
    }

    /// `context.close()`: closes its pages and the user context.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner
            .bidi
            .call(
                "browser.removeUserContext",
                json!({"userContext": self.inner.user_context}),
            )
            .await?;
        Ok(())
    }
}

/// A Firefox page (a top-level browsing context in its own window).
#[derive(Clone)]
pub struct Page {
    inner: Arc<PageInner>,
}

struct PageInner {
    bidi: Arc<dyn BidiSession>,
    services: Services,
    /// The browsing context id.
    context: String,
    options: Arc<ContextOptions>,
    /// `set_viewport_size` (else the context's viewport).
    viewport: Mutex<Option<Viewport>>,
    /// The context `browser.new_page()` made for this page.
    owned_context: Mutex<Option<BrowserContext>>,
    /// The main frame's URL, from `browsingContext` events.
    url: Mutex<String>,
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
    fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("page has been closed".into()));
        }
        Ok(())
    }

    /// `page.goto(url)`: navigates and waits for the `load` event.
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.check_open()?;
        self.inner
            .bidi
            .call(
                "browsingContext.navigate",
                json!({"context": self.inner.context, "url": url, "wait": "complete"}),
            )
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        Ok(())
    }

    /// `page.url()`: the main frame's URL.
    pub fn url(&self) -> String {
        self.inner.url.lock().unwrap().clone()
    }

    /// `page.content()`: the doctype and `documentElement.outerHTML`.
    pub async fn content(&self) -> Result<String> {
        let v = self
            .call_json(
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
        self.traverse(-1).await
    }

    /// `page.goForward()` (nothing happens when there is no next entry).
    pub async fn go_forward(&self) -> Result<()> {
        self.traverse(1).await
    }

    /// `page.waitForTimeout(ms)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.sleep(duration).await
    }

    /// `page.waitForFunction(fn | expression)`: polls until it returns a
    /// truthy value (Playwright's 30 s default timeout) and resolves with it.
    pub async fn wait_for_function(&self, source: &str) -> Result<Value> {
        let timeout = Duration::from_secs(30);
        let deadline = self.now() + timeout;
        loop {
            match self.evaluate_value(source, None).await {
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
        let r = self.script_evaluate(&wrapped, None).await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            let message = exception_text(&r);
            if !message.starts_with("SyntaxError") {
                return Err(Error::Evaluation(message));
            }
            let r = self.script_evaluate(source, None).await?;
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
        self.check_open()?;
        let args: Vec<String> = args.iter().map(Value::to_string).collect();
        let expression = format!(
            "(async () => {{ const __r = await ({function})({}); \
             return __r === undefined ? undefined : JSON.stringify(__r); }})()",
            args.join(", ")
        );
        let r = self.script_evaluate(&expression, sandbox).await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        match r.pointer("/result/value").and_then(Value::as_str) {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    async fn script_evaluate(&self, expression: &str, sandbox: Option<&str>) -> Result<Value> {
        let mut target = json!({"context": self.inner.context});
        if let Some(sandbox) = sandbox {
            target["sandbox"] = json!(sandbox);
        }
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
    pub(crate) async fn insert_text(&self, text: &str) -> Result<()> {
        self.call_json(
            "(text) => { document.execCommand('insertText', false, text); }",
            &[json!(text)],
            None,
        )
        .await?;
        Ok(())
    }

    pub(crate) fn now(&self) -> Duration {
        Duration::from_millis(self.inner.services.clock.now_ms())
    }

    pub(crate) async fn sleep(&self, duration: Duration) {
        self.inner.services.clock.sleep(duration).await
    }

    /// Calls `body(injected, ...args)` in the utility sandbox, installing
    /// Playwright's injected script first when the document has none yet.
    pub(crate) async fn injected_call(&self, body: &str, args: &[Value]) -> Result<Value> {
        let call = format!(
            "(...args) => {{ const i = globalThis.{INJECTED}; \
             if (!i) return {{ __chromiumoxide_missing: true }}; return ({body})(i, ...args); }}"
        );
        let v = self.call_json(&call, args, Some(UTILITY)).await?;
        if v.get("__chromiumoxide_missing") != Some(&Value::Bool(true)) {
            return Ok(v);
        }
        let options = json!({
            "isUnderTest": false, "sdkLanguage": "javascript", "frameSeq": 0,
            "testIdAttributeName": "data-testid", "stableRafCount": 1, "browserName": "firefox",
            "isUtilityWorld": true, "customEngines": [],
        });
        let install = format!(
            "(() => {{ if (globalThis.{INJECTED}) return; const module = {{}};\n{INJECTED_SOURCE}\n\
             globalThis.{INJECTED} = new (module.exports.InjectedScript())(globalThis, {options}); }})()"
        );
        let r = self.script_evaluate(&install, Some(UTILITY)).await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        self.call_json(&call, args, Some(UTILITY)).await
    }

    /// `page.setContent(html)`: replaces the document and waits for `load`
    /// (Playwright's `document.open/write/close`).
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.call_json(
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
        self.call_json(ADD_STYLE, &[content, url], None).await?;
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
fn exception_text(r: &Value) -> String {
    let details = &r["exceptionDetails"];
    details
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| remote_value_to_json(&details["exception"]).to_string())
}
