//! The Firefox engine: the Playwright API over WebDriver BiDi
//! (`chromiumoxide::firefox`). See `docs/ARCHITECTURE.md`, "Engines".

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::services::Services;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::{command_line, remote_value_to_json};
use crate::ports::{BidiSession, BrowserProcess, FirefoxCommand};

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

    /// `browser.newPage()`: a page in a new user context (closed with it).
    pub async fn new_page(&self) -> Result<Page> {
        let user_context = self
            .inner
            .bidi
            .call("browser.createUserContext", json!({}))
            .await?
            .get("userContext")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let created = self
            .inner
            .bidi
            .call(
                "browsingContext.create",
                // A foreground tab never finishes opening in headless
                // Firefox (it waits for focus); a window does, like
                // Playwright's one window per page.
                json!({"type": "window", "userContext": user_context}),
            )
            .await?;
        let context = created
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Page {
            inner: Arc::new(PageInner {
                bidi: self.inner.bidi.clone(),
                context,
                owned_user_context: Some(user_context),
                closed: AtomicBool::new(false),
            }),
        })
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

/// A Firefox page (a top-level browsing context in its own window).
#[derive(Clone)]
pub struct Page {
    inner: Arc<PageInner>,
}

struct PageInner {
    bidi: Arc<dyn BidiSession>,
    /// The browsing context id.
    context: String,
    /// The user context created for this page by `browser.new_page()`.
    owned_user_context: Option<String>,
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
        let r = self.script_evaluate(&wrapped).await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            let message = exception_text(&r);
            if !message.starts_with("SyntaxError") {
                return Err(Error::Evaluation(message));
            }
            let r = self.script_evaluate(source).await?;
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

    async fn script_evaluate(&self, expression: &str) -> Result<Value> {
        self.inner
            .bidi
            .call(
                "script.evaluate",
                json!({
                    "expression": expression,
                    "target": {"context": self.inner.context},
                    "awaitPromise": true,
                    "resultOwnership": "none",
                    "userActivation": true,
                }),
            )
            .await
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
        if let Some(user_context) = &self.inner.owned_user_context {
            let _ = self
                .inner
                .bidi
                .call(
                    "browser.removeUserContext",
                    json!({"userContext": user_context}),
                )
                .await;
        }
        Ok(())
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
