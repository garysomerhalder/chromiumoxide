//! Playwright's `_electron`: Electron apps. The windows are pages of the
//! app's default context (over CDP); the main process is reached through
//! its Node inspector, where `require('electron')` is the module handed to
//! `evaluate`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::browser::Browser;
use crate::application::context::{BrowserContext, ContextEvent};
use crate::application::page::Page;
use crate::application::services::Services;
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};
use crate::ports::{CdpSession, ElectronCommand};

/// Vendored from playwright-core 1.65.1: holds back the app's `ready` event
/// until `__playwright_run()`, so no window is missed.
const LOADER: &str = include_str!("scripts/electron_loader.js");

/// Playwright's `electron.launch(options)`.
#[derive(Debug, Clone)]
pub struct ElectronLaunchOptions {
    /// The Electron binary or a packaged app. Without it, Electron is found
    /// like `require('electron')` and Playwright's loader is preloaded.
    pub executable_path: Option<PathBuf>,
    /// Arguments for the app, e.g. the path of its main script.
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// The app's whole environment (`None`: this process's).
    pub env: Option<Vec<(String, String)>>,
    /// How long to wait for the app to start (default 30 s).
    pub timeout: Duration,
    /// Options of the app's context. There is never a default viewport:
    /// pages are as big as their windows.
    pub context: ContextOptions,
    /// Keep Chromium's sandbox (Linux; default `false`: `--no-sandbox`).
    pub chromium_sandbox: bool,
}

impl Default for ElectronLaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            args: Vec::new(),
            cwd: None,
            env: None,
            timeout: Duration::from_secs(30),
            context: ContextOptions::default(),
            chromium_sandbox: false,
        }
    }
}

/// Something that happened to an Electron app.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ElectronEvent {
    /// A window opened (`electronApp.on('window')`).
    Window(Page),
    /// The main process logged (`electronApp.on('console')`).
    Console(ElectronConsoleMessage),
    /// The app exited (`electronApp.on('close')`).
    Close,
}

/// A main-process console message.
#[derive(Debug, Clone, PartialEq)]
pub struct ElectronConsoleMessage {
    kind: String,
    text: String,
    args: Vec<Value>,
}

impl ElectronConsoleMessage {
    /// `log`, `error`, `warning`, ...
    pub fn kind(&self) -> &str {
        &self.kind
    }
    /// The arguments as text, joined by spaces.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// The arguments as protocol remote objects.
    pub fn args(&self) -> &[Value] {
        &self.args
    }
}

/// The app's process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ElectronProcess {
    pid: u32,
}

impl ElectronProcess {
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

struct Inner {
    browser: Browser,
    context: BrowserContext,
    node: Arc<dyn CdpSession>,
    /// `require('electron')` in the main process.
    electron: String,
    pid: u32,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<ElectronEvent>>>,
    services: Services,
    loader_dir: Option<PathBuf>,
}

impl Inner {
    fn emit(&self, event: ElectronEvent) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }
}

/// Playwright's `ElectronApplication`. Cheap to clone.
#[derive(Clone)]
pub struct ElectronApplication {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ElectronApplication {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElectronApplication")
            .field("pid", &self.inner.pid)
            .finish()
    }
}

impl ElectronApplication {
    /// `electron.launch(options)`. `found` is the Electron binary located like
    /// `require('electron')` (used when `executable_path` is not set).
    pub async fn launch(
        services: Services,
        options: ElectronLaunchOptions,
        found: Option<PathBuf>,
    ) -> Result<ElectronApplication> {
        options.context.validate()?;
        let mut args = vec![
            "--inspect=0".to_owned(),
            "--remote-debugging-port=0".to_owned(),
        ];
        args.extend(options.args.iter().cloned());
        if cfg!(target_os = "linux")
            && !options.chromium_sandbox
            && !args.iter().any(|a| a == "--no-sandbox")
        {
            args.insert(0, "--no-sandbox".into());
        }
        let mut loader_dir = None;
        let program = match options.executable_path.clone() {
            Some(p) => p,
            None => {
                let exe = found.ok_or_else(|| {
                    Error::Launch(
                        "Electron executablePath not found! Install it with `npm install -D electron` \
                         or set `executable_path` to your Electron executable."
                            .into(),
                    )
                })?;
                let dir = services.fs.create_temp_dir("chromiumoxide-electron-")?;
                let loader = dir.join("loader.js");
                services.fs.write(&loader, LOADER.as_bytes())?;
                args.insert(0, loader.to_string_lossy().into_owned());
                args.insert(0, "-r".into());
                loader_dir = Some(dir);
                exe
            }
        };
        let command = ElectronCommand {
            program,
            args,
            env: options.env.clone(),
            cwd: options.cwd.clone(),
            timeout_ms: options.timeout.as_millis() as u64,
        };
        let cleanup = |services: &Services, dir: &Option<PathBuf>| {
            if let Some(d) = dir {
                let _ = services.fs.remove_dir_all(d);
            }
        };
        let launched = match services.launcher.launch_electron(command).await {
            Ok(l) => l,
            Err(e) => {
                cleanup(&services, &loader_dir);
                return Err(e);
            }
        };
        let pid = launched.process.pid().unwrap_or_default();
        let node = launched.node;
        // Subscribe before `Runtime.enable` reports the existing contexts.
        let mut node_events = node.subscribe();
        let context_options = ContextOptions {
            viewport: None,
            ..options.context
        };
        let electron = async {
            node.call(None, "Runtime.enable", json!({})).await?;
            let context_id = loop {
                let Some(e) = node_events.next().await else {
                    return Err(Error::Launch("Electron: the main process went away".into()));
                };
                if e.method == "Runtime.executionContextCreated"
                    && e.params.pointer("/context/auxData/isDefault") == Some(&json!(true))
                {
                    break e.params["context"]["id"].clone();
                }
            };
            // `includeCommandLineAPI` gives `require` (Electron 28+).
            let r = node
                .call(
                    None,
                    "Runtime.evaluate",
                    json!({"expression": "require('electron')", "contextId": context_id,
                           "includeCommandLineAPI": true}),
                )
                .await?;
            crate::application::js::result_value(&r)?;
            r.pointer("/result/objectId")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| Error::Launch("Electron: require('electron') failed".into()))
        };
        let electron = crate::application::wait::with_timeout(
            &*services.clock,
            options.timeout,
            "waiting for the Electron main process",
            electron,
        )
        .await;
        let electron = match electron {
            Ok(id) => id,
            Err(e) => {
                launched.process.shutdown().await;
                cleanup(&services, &loader_dir);
                return Err(e);
            }
        };
        let (browser, context) = match Browser::from_electron(
            services.clone(),
            launched.browser,
            launched.process,
            context_options,
        )
        .await
        {
            Ok(b) => b,
            Err(e) => {
                cleanup(&services, &loader_dir);
                return Err(e);
            }
        };
        let app = ElectronApplication {
            inner: Arc::new(Inner {
                browser,
                context,
                node: node.clone(),
                electron,
                pid,
                subscribers: Mutex::new(Vec::new()),
                services,
                loader_dir,
            }),
        };
        app.forward_events(node_events);
        // Lets the app's `ready` through (a no-op without the loader).
        let _ = node
            .call(
                None,
                "Runtime.evaluate",
                json!({"expression": "globalThis.__playwright_run && __playwright_run()",
                       "awaitPromise": true}),
            )
            .await;
        Ok(app)
    }

    /// Main-process console messages and the exit, plus new windows.
    fn forward_events(&self, mut node_events: crate::ports::EventStream) {
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            while let Some(e) = node_events.next().await {
                if e.method != "Runtime.consoleAPICalled"
                    || e.params.get("executionContextId") == Some(&json!(0))
                {
                    continue;
                }
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let args: Vec<Value> = e.params["args"].as_array().cloned().unwrap_or_default();
                inner.emit(ElectronEvent::Console(ElectronConsoleMessage {
                    kind: e.params["type"].as_str().unwrap_or("log").to_owned(),
                    text: crate::domain::console::console_text(&args),
                    args,
                }));
            }
            // The inspector connection ends with the process.
            if let Some(inner) = weak.upgrade() {
                inner.emit(ElectronEvent::Close);
            }
        });
        let mut context_events = self.inner.context.events();
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            while let Some(e) = context_events.next().await {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                match e {
                    ContextEvent::Page(page) => inner.emit(ElectronEvent::Window(page)),
                    ContextEvent::Close => return,
                    _ => {}
                }
            }
        });
    }

    /// `electronApp.on(...)`: this app's events from now on.
    pub fn events(&self) -> impl futures::Stream<Item = ElectronEvent> + Send + use<> {
        let (tx, rx) = mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// `electronApp.context()`: the context of the app's windows.
    pub fn context(&self) -> BrowserContext {
        self.inner.context.clone()
    }

    /// `electronApp.windows()`: the open windows.
    pub fn windows(&self) -> Vec<Page> {
        self.inner
            .context
            .pages()
            .into_iter()
            .filter(|p| !p.is_closed())
            .collect()
    }

    /// `electronApp.firstWindow()`: the first window, waiting for it if
    /// none is open yet (default timeout 30 s).
    pub async fn first_window(&self) -> Result<Page> {
        self.first_window_with_timeout(Duration::from_secs(30))
            .await
    }

    /// `electronApp.firstWindow({ timeout })`.
    pub async fn first_window_with_timeout(&self, timeout: Duration) -> Result<Page> {
        let mut events = self.inner.context.events();
        if let Some(p) = self.windows().into_iter().next() {
            return Ok(p);
        }
        let wait = async {
            while let Some(e) = events.next().await {
                if let ContextEvent::Page(p) = e {
                    return Ok(p);
                }
            }
            Err(Error::TargetClosed("Electron application closed".into()))
        };
        crate::application::wait::with_timeout(
            &*self.inner.services.clock,
            timeout,
            "electronApplication.firstWindow",
            wait,
        )
        .await
    }

    /// `electronApp.evaluate(fn)`: `fn(electron)` in the main process.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.call_on(&self.inner.electron, function, None, true)
            .await
            .and_then(|v| Ok(serde_json::from_value(v)?))
    }

    /// `electronApp.evaluate(fn, arg)`: `fn(electron, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        self.call_on(&self.inner.electron, function, Some(arg), true)
            .await
            .and_then(|v| Ok(serde_json::from_value(v)?))
    }

    /// `electronApp.evaluateHandle(fn, arg)`: the result as a handle.
    pub async fn evaluate_handle(
        &self,
        function: &str,
        arg: Option<Value>,
    ) -> Result<ElectronHandle> {
        let r = self
            .call_on(&self.inner.electron, function, arg, false)
            .await?;
        Ok(ElectronHandle {
            app: self.clone(),
            remote: r,
        })
    }

    /// `electronApp.browserWindow(page)`: the `BrowserWindow` of a window.
    pub async fn browser_window(&self, page: &Page) -> Result<ElectronHandle> {
        self.evaluate_handle(
            "({ BrowserWindow, webContents }, targetId) => \
             BrowserWindow.fromWebContents(webContents.fromDevToolsTargetId(targetId))",
            Some(json!(page.target_id())),
        )
        .await
    }

    /// `electronApp.process()`.
    pub fn process(&self) -> ElectronProcess {
        ElectronProcess {
            pid: self.inner.pid,
        }
    }

    /// `electronApp.close()`: quits the app and waits for it to exit.
    pub async fn close(&self) -> Result<()> {
        let _ = self.evaluate::<Value>("({ app }) => { app.quit(); }").await;
        if let Some(process) = self.inner.browser.take_process() {
            process.shutdown().await;
        }
        self.inner.node.disconnect();
        let _ = self.inner.browser.close().await;
        if let Some(dir) = &self.inner.loader_dir {
            let _ = self.inner.services.fs.remove_dir_all(dir);
        }
        Ok(())
    }

    /// `functionDeclaration` called on `object_id` with `this` as the first
    /// argument; by value, or the remote object.
    async fn call_on(
        &self,
        object_id: &str,
        function: &str,
        arg: Option<Value>,
        by_value: bool,
    ) -> Result<Value> {
        let declaration = match arg {
            Some(_) => format!("function(arg) {{ return ({function})(this, arg); }}"),
            None => format!("function() {{ return ({function})(this); }}"),
        };
        let arguments: Vec<Value> = arg.into_iter().map(|v| json!({"value": v})).collect();
        let r = self
            .inner
            .node
            .call(
                None,
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": declaration, "objectId": object_id,
                       "arguments": arguments, "returnByValue": by_value, "awaitPromise": true}),
            )
            .await?;
        if by_value {
            crate::application::js::result_value(&r)
        } else {
            if r.get("exceptionDetails").is_some() {
                crate::application::js::result_value(&r)?;
            }
            Ok(r["result"].clone())
        }
    }
}

/// A main-process object (`electronApp.evaluateHandle`, `browserWindow`).
#[derive(Clone, Debug)]
pub struct ElectronHandle {
    app: ElectronApplication,
    /// The protocol remote object.
    remote: Value,
}

impl ElectronHandle {
    /// `handle.evaluate(fn)`: `fn(object)` in the main process.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        let v = match self.object_id() {
            Some(id) => self.app.call_on(id, function, None, true).await?,
            None => {
                let value = self.remote.get("value").cloned().unwrap_or(Value::Null);
                self.app
                    .call_on(
                        &self.app.inner.electron,
                        &format!("(_, v) => ({function})(v)"),
                        Some(value),
                        true,
                    )
                    .await?
            }
        };
        Ok(serde_json::from_value(v)?)
    }

    /// `handle.jsonValue()`.
    pub async fn json_value<T: DeserializeOwned>(&self) -> Result<T> {
        self.evaluate("o => o").await
    }

    /// `handle.dispose()`.
    pub async fn dispose(&self) -> Result<()> {
        if let Some(id) = self.object_id() {
            let _ = self
                .app
                .inner
                .node
                .call(None, "Runtime.releaseObject", json!({"objectId": id}))
                .await;
        }
        Ok(())
    }

    fn object_id(&self) -> Option<&str> {
        self.remote.get("objectId").and_then(Value::as_str)
    }
}
