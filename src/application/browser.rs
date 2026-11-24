//! Use cases: launch a browser, query and close it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::collections::HashMap;
use std::sync::Weak;

use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::context::BrowserContext;
use crate::application::discovery::discover;
use crate::application::page::Page;
use crate::application::services::Services;
use crate::application::shared::Shared;
use crate::domain::emulation::{ContextOptions, FontPlatform};
use crate::domain::error::{Error, Result};
use crate::domain::launch::{LaunchSpec, command_line};
use crate::ports::{BrowserProcess, CdpSession, Channel, EventStream, LaunchCommand};

/// Playwright's `browserType.launch(options)`.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchOptions {
    /// Use this executable instead of discovering one.
    pub executable_path: Option<PathBuf>,
    /// Headless (default `true`).
    pub headless: bool,
    /// Extra command-line switches.
    pub args: Vec<String>,
    /// Drop the default argument preset.
    pub ignore_default_args: bool,
    /// Drop only these default switches, e.g. `--disable-extensions`
    /// (Playwright's `ignoreDefaultArgs: [...]`).
    pub ignored_default_args: Vec<String>,
    /// Extra environment variables for the browser process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the browser to start (default 180 s, like Playwright).
    pub timeout: Duration,
    /// Pipe (default) or WebSocket.
    pub channel: Channel,
    /// Route the browser's traffic through a proxy (Playwright's `proxy`).
    pub proxy: Option<crate::domain::launch::Proxy>,
    /// Playwright's `slowMo`: wait this long after each action, navigation
    /// and keyboard, mouse or touch call.
    pub slow_mo: Duration,
    /// Playwright's `downloadsPath`: accepted downloads go here (not removed
    /// on close; the files still go with their context).
    pub downloads_path: Option<PathBuf>,
    /// Playwright's `tracesDir`: tracing also leaves `<name>.trace` and
    /// `<name>.network` here.
    pub traces_dir: Option<PathBuf>,
    /// Playwright's `artifactsDir`: the default for `downloads_path` and
    /// `traces_dir`; not removed on close.
    pub artifacts_dir: Option<PathBuf>,
    /// Playwright's `chromiumSandbox` (default `false`: `--no-sandbox`).
    pub chromium_sandbox: bool,
    /// Playwright's `handleSIGINT`: on Ctrl-C, close the browser (a second
    /// Ctrl-C kills it) and exit with 130. Off by default, unlike
    /// Playwright: listening replaces the program's own Ctrl-C behaviour.
    pub handle_sigint: bool,
    /// Playwright's `handleSIGTERM`: on SIGTERM, close the browser; the
    /// program goes on. Off by default (see `handle_sigint`).
    pub handle_sigterm: bool,
    /// Playwright's `handleSIGHUP`: like `handle_sigterm`, for SIGHUP.
    pub handle_sighup: bool,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            headless: true,
            args: Vec::new(),
            ignore_default_args: false,
            ignored_default_args: Vec::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(180),
            channel: Channel::Pipe,
            proxy: None,
            slow_mo: Duration::ZERO,
            downloads_path: None,
            traces_dir: None,
            artifacts_dir: None,
            chromium_sandbox: false,
            handle_sigint: false,
            handle_sigterm: false,
            handle_sighup: false,
        }
    }
}

/// Playwright's `browser.bind(title, options)` options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindOptions {
    /// Identifies the browser in [`Chromium::bound_browsers`](crate::Chromium::bound_browsers).
    pub title: String,
    /// Default `127.0.0.1`.
    pub host: Option<String>,
    /// Default `0`: any free port.
    pub port: Option<u16>,
    /// The working directory this browser belongs to.
    pub workspace_dir: Option<PathBuf>,
    /// Anything else to record (`null` if none).
    pub metadata: serde_json::Value,
}

/// A connected browser.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<BrowserInner>,
}

pub(crate) struct BrowserInner {
    cdp: Arc<dyn CdpSession>,
    services: Services,
    shared: Mutex<Option<Arc<Shared>>>,
    headless: bool,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
    user_data_dir: Option<PathBuf>,
    executable: Option<PathBuf>,
    ws_endpoint: Option<String>,
    closed: AtomicBool,
    /// Session of `browser.startTracing` (`None` inside: the browser session).
    tracing: Mutex<Option<Option<String>>>,
    /// Delete the profile on close (temporary profiles only).
    remove_profile: bool,
    /// `browser.bind()`: the connection served to other clients.
    bridge: Mutex<Option<Box<dyn crate::ports::BoundBridge>>>,
    /// The registry descriptor written by `bind` (removed by `unbind`).
    registry_file: Mutex<Option<PathBuf>>,
    /// `LaunchOptions::args` (None when connected).
    launch_args: Option<Vec<String>>,
    /// `LaunchOptions::proxy`.
    launch_proxy: Option<crate::domain::launch::Proxy>,
    /// `slowMo`, `downloadsPath`, `tracesDir` (`artifactsDir` resolved).
    artifacts: crate::application::shared::LaunchArtifacts,
    /// `browser.events()` subscribers.
    subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<BrowserEvent>>>,
    /// `disconnected` was emitted.
    disconnected: AtomicBool,
}

/// Something that happened to a browser (Playwright's `browser.on(...)`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum BrowserEvent {
    /// A context was created (`browser.on('context')`).
    Context(BrowserContext),
    /// The browser closed or the connection was lost (`browser.on('disconnected')`).
    Disconnected,
}

impl BrowserInner {
    fn emit(&self, event: BrowserEvent) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }

    /// Emits `disconnected` once.
    pub(crate) fn emit_disconnected(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if !self.disconnected.swap(true, Ordering::SeqCst) {
            self.emit(BrowserEvent::Disconnected);
        }
    }
}

impl Drop for BrowserInner {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::SeqCst) {
            // Dropping the process kills it; then the profile can go.
            drop(self.process.lock().unwrap().take());
            if let Some(dir) = self.user_data_dir.as_ref().filter(|_| self.remove_profile) {
                let _ = self.services.fs.remove_dir_all(dir);
            }
        }
    }
}

/// Launched browsers that handle SIGINT and are still open: the program
/// exits once the last of them has closed (Playwright closes all, then exits).
static SIGINT_BROWSERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Browser {
    /// `handleSIGINT` / `handleSIGTERM` / `handleSIGHUP`: one task per signal,
    /// holding the browser weakly, until the signal or the disconnect.
    fn handle_signals(&self, options: &LaunchOptions) {
        use crate::ports::Signal;
        let wanted = [
            (options.handle_sigint, Signal::Interrupt),
            (options.handle_sigterm, Signal::Terminate),
            (options.handle_sighup, Signal::Hangup),
        ];
        for (_, signal) in wanted.into_iter().filter(|(on, _)| *on) {
            let signals = self.inner.services.signals.clone();
            let mut delivered = signals.listen(signal);
            let mut events = self.events();
            let weak = Arc::downgrade(&self.inner);
            let interrupt = signal == Signal::Interrupt;
            if interrupt {
                SIGINT_BROWSERS.fetch_add(1, Ordering::SeqCst);
            }
            tokio::spawn(async move {
                use futures::StreamExt;
                let got = loop {
                    tokio::select! {
                        s = delivered.next() => break s.is_some(),
                        e = events.next() => match e {
                            Some(BrowserEvent::Disconnected) | None => break false,
                            Some(_) => {}
                        },
                    }
                };
                let browser = weak.upgrade().map(|inner| Browser { inner });
                let Some(browser) = browser.filter(|_| got) else {
                    if interrupt {
                        SIGINT_BROWSERS.fetch_sub(1, Ordering::SeqCst);
                    }
                    return;
                };
                if !interrupt {
                    let _ = browser.close().await;
                    return;
                }
                tokio::select! {
                    _ = browser.close() => {}
                    // A second Ctrl-C kills the browser right away.
                    _ = delivered.next() => browser.kill_now(),
                }
                if SIGINT_BROWSERS.fetch_sub(1, Ordering::SeqCst) == 1 {
                    signals.exit(130);
                }
            });
        }
    }

    /// Kills the process without closing the connection, removes a
    /// temporary profile.
    fn kill_now(&self) {
        drop(self.inner.process.lock().unwrap().take());
        if let Some(dir) = self
            .inner
            .user_data_dir
            .as_ref()
            .filter(|_| self.inner.remove_profile)
        {
            let _ = self.inner.services.fs.remove_dir_all(dir);
        }
        self.inner.emit_disconnected();
    }

    /// Discovers, launches and connects to a browser.
    pub async fn launch(services: Services, options: LaunchOptions) -> Result<Browser> {
        Ok(Self::launch_impl(services, options, None).await?.0)
    }

    /// `chromium.launchPersistentContext(userDataDir, options)`: a browser on a
    /// kept profile; its default context is returned. Closing the context
    /// closes the browser.
    pub async fn launch_persistent(
        services: Services,
        user_data_dir: PathBuf,
        options: LaunchOptions,
        context: ContextOptions,
    ) -> Result<BrowserContext> {
        context.validate()?;
        let (browser, ctx) =
            Self::launch_impl(services, options, Some((user_data_dir, context))).await?;
        let ctx = ctx.expect("persistent launches have a default context");
        *ctx.inner.owner.lock().unwrap() = Some(browser);
        Ok(ctx)
    }

    async fn launch_impl(
        services: Services,
        options: LaunchOptions,
        persistent: Option<(PathBuf, ContextOptions)>,
    ) -> Result<(Browser, Option<BrowserContext>)> {
        let found = discover(
            &*services.fs,
            &*services.env,
            options.executable_path.as_deref(),
        )?;
        let user_data_dir = match &persistent {
            Some((dir, _)) => dir.clone(),
            None => services.fs.create_temp_dir("chromiumoxide-profile-")?,
        };
        let spec = LaunchSpec {
            kind: found.kind,
            headless: options.headless,
            args: options.args.clone(),
            ignore_default_args: options.ignore_default_args,
            ignored_default_args: options.ignored_default_args.clone(),
            user_data_dir: user_data_dir.clone(),
            proxy: options.proxy.clone(),
            chromium_sandbox: options.chromium_sandbox,
        };
        let mut args = if persistent.is_some() {
            crate::domain::launch::persistent_command_line(&spec)
        } else {
            command_line(&spec)
        };
        if options.channel == Channel::WebSocket {
            for a in &mut args {
                if a == "--remote-debugging-pipe" {
                    *a = "--remote-debugging-port=0".to_owned();
                }
            }
        }
        let command = LaunchCommand {
            program: found.executable.clone(),
            args,
            env: options.env.clone(),
            channel: options.channel,
            user_data_dir: user_data_dir.clone(),
            timeout_ms: options.timeout.as_millis() as u64,
        };
        let launched = match services.launcher.launch(command).await {
            Ok(l) => l,
            Err(e) => {
                if persistent.is_none() {
                    let _ = services.fs.remove_dir_all(&user_data_dir);
                }
                return Err(e);
            }
        };
        let browser = Browser {
            inner: Arc::new(BrowserInner {
                cdp: launched.cdp,
                services,
                shared: Mutex::new(None),
                headless: options.headless,
                process: Mutex::new(Some(launched.process)),
                user_data_dir: Some(user_data_dir),
                executable: Some(found.executable),
                ws_endpoint: launched.ws_endpoint,
                closed: AtomicBool::new(false),
                tracing: Mutex::new(None),
                remove_profile: persistent.is_none(),
                bridge: Mutex::new(None),
                registry_file: Mutex::new(None),
                launch_args: Some(options.args.clone()),
                launch_proxy: options.proxy.clone(),
                artifacts: crate::application::shared::LaunchArtifacts {
                    slow_mo: options.slow_mo,
                    downloads_path: options
                        .downloads_path
                        .clone()
                        .or_else(|| options.artifacts_dir.clone()),
                    traces_dir: options
                        .traces_dir
                        .clone()
                        .or_else(|| options.artifacts_dir.clone()),
                },
                subscribers: Mutex::new(Vec::new()),
                disconnected: AtomicBool::new(false),
            }),
        };
        let default_options = persistent.map(|(_, o)| o);
        match browser.handshake(default_options).await {
            Ok(ctx) => {
                browser.handle_signals(&options);
                Ok((browser, ctx))
            }
            Err(e) => {
                let _ = browser.close().await;
                Err(e)
            }
        }
    }

    /// Attaches to an already running browser (`connectOverCDP`). Closing it
    /// closes the browser.
    pub async fn connect(
        services: Services,
        cdp: Arc<dyn CdpSession>,
        ws_endpoint: Option<String>,
    ) -> Result<Browser> {
        let browser = Browser {
            inner: Arc::new(BrowserInner {
                cdp,
                services,
                shared: Mutex::new(None),
                headless: true,
                process: Mutex::new(None),
                user_data_dir: None,
                executable: None,
                ws_endpoint,
                closed: AtomicBool::new(false),
                tracing: Mutex::new(None),
                remove_profile: false,
                bridge: Mutex::new(None),
                registry_file: Mutex::new(None),
                launch_args: None,
                launch_proxy: None,
                artifacts: Default::default(),
                subscribers: Mutex::new(Vec::new()),
                disconnected: AtomicBool::new(false),
            }),
        };
        browser.handshake(None).await?;
        Ok(browser)
    }

    /// An Electron app's DevTools connection: a browser whose default
    /// context (with `context` options) holds the app's windows.
    pub(crate) async fn from_electron(
        services: Services,
        cdp: Arc<dyn CdpSession>,
        process: Box<dyn BrowserProcess>,
        context: ContextOptions,
    ) -> Result<(Browser, BrowserContext)> {
        Self::with_default_context(services, cdp, Some(process), context).await
    }

    /// A browser on an existing CDP connection whose default context (with
    /// `context` options) is the one to use: Electron apps, Chrome and
    /// WebViews on Android. Closing that context closes the connection.
    pub(crate) async fn with_default_context(
        services: Services,
        cdp: Arc<dyn CdpSession>,
        process: Option<Box<dyn BrowserProcess>>,
        context: ContextOptions,
    ) -> Result<(Browser, BrowserContext)> {
        let browser = Browser {
            inner: Arc::new(BrowserInner {
                cdp,
                services,
                shared: Mutex::new(None),
                headless: false,
                process: Mutex::new(process),
                user_data_dir: None,
                executable: None,
                ws_endpoint: None,
                closed: AtomicBool::new(false),
                tracing: Mutex::new(None),
                remove_profile: false,
                bridge: Mutex::new(None),
                registry_file: Mutex::new(None),
                launch_args: None,
                launch_proxy: None,
                artifacts: Default::default(),
                subscribers: Mutex::new(Vec::new()),
                disconnected: AtomicBool::new(false),
            }),
        };
        let ctx = browser
            .handshake(Some(context))
            .await?
            .ok_or_else(|| Error::Launch("no default context".into()))?;
        Ok((browser, ctx))
    }

    /// Takes the launched process (the caller ends it).
    pub(crate) fn take_process(&self) -> Option<Box<dyn BrowserProcess>> {
        self.inner.process.lock().unwrap().take()
    }

    /// Playwright's `browser.bind()`: lets other clients
    /// ([`Chromium::connect`](crate::Chromium::connect)) share this browser
    /// over a local WebSocket, whatever channel it was launched with. Their
    /// endpoint speaks CDP. Binding again returns the same endpoint.
    pub async fn bind(&self) -> Result<String> {
        self.bind_with(BindOptions::default()).await
    }

    /// Playwright's `browser.bind(title, { host, port, workspaceDir,
    /// metadata })`: like [`bind`](Self::bind), listening on `host:port`,
    /// and listed by [`Chromium::bound_browsers`](crate::Chromium::bound_browsers)
    /// until unbound or closed.
    pub async fn bind_with(&self, options: BindOptions) -> Result<String> {
        let address = crate::ports::BridgeAddress {
            host: options.host.clone(),
            port: options.port,
            path: None,
        };
        let endpoint = self.serve(address).await?;
        let services = &self.inner.services;
        let dir = crate::domain::registry::registry_dir(
            services.env.os(),
            |k| services.env.var(k),
            services.env.home_dir().as_deref(),
        );
        if let Some(dir) = dir {
            let descriptor = crate::domain::registry::BoundBrowser {
                title: options.title,
                endpoint: endpoint.clone(),
                workspace_dir: options.workspace_dir,
                metadata: options.metadata,
            };
            let file = dir.join(descriptor.file_name());
            services.fs.write(&file, descriptor.to_json().as_bytes())?;
            *self.inner.registry_file.lock().unwrap() = Some(file);
        }
        Ok(endpoint)
    }

    /// Serves the connection at `address` (the endpoint, if already served).
    pub(crate) async fn serve(&self, address: crate::ports::BridgeAddress) -> Result<String> {
        if let Some(b) = self.inner.bridge.lock().unwrap().as_ref() {
            return Ok(b.endpoint());
        }
        let bound = self
            .inner
            .services
            .bridge
            .serve(self.inner.cdp.clone(), address)
            .await?;
        let endpoint = bound.endpoint();
        *self.inner.bridge.lock().unwrap() = Some(bound);
        Ok(endpoint)
    }

    /// `browser.unbind()`: stops serving other clients.
    pub async fn unbind(&self) -> Result<()> {
        if let Some(b) = self.inner.bridge.lock().unwrap().take() {
            b.stop();
        }
        if let Some(file) = self.inner.registry_file.lock().unwrap().take() {
            let _ = self.inner.services.fs.remove_file(&file);
        }
        Ok(())
    }

    /// The browsers bound with [`bind_with`](Self::bind_with) on this
    /// machine that still accept clients (stale descriptors are removed).
    pub async fn bound_browsers(
        services: &Services,
    ) -> Result<Vec<crate::domain::registry::BoundBrowser>> {
        let dir = crate::domain::registry::registry_dir(
            services.env.os(),
            |k| services.env.var(k),
            services.env.home_dir().as_deref(),
        );
        let Some(dir) = dir else {
            return Ok(Vec::new());
        };
        let mut found = Vec::new();
        for name in services.fs.list_dir(&dir) {
            let path = dir.join(&name);
            let Some(b) = services.fs.read(&path).ok().and_then(|t| {
                crate::domain::registry::BoundBrowser::from_json(&String::from_utf8_lossy(&t))
            }) else {
                continue;
            };
            if services.bridge.is_listening(&b.endpoint).await {
                found.push(b);
            } else {
                let _ = services.fs.remove_file(&path);
            }
        }
        Ok(found)
    }

    /// The DevTools WebSocket URL, if the browser was launched or connected over one.
    pub fn ws_endpoint(&self) -> Option<String> {
        self.inner.ws_endpoint.clone()
    }

    /// Playwright's connection handshake on the browser target, and the
    /// event loop that resumes or initialises auto-attached targets.
    async fn handshake(
        &self,
        default_context: Option<ContextOptions>,
    ) -> Result<Option<BrowserContext>> {
        let cdp = &self.inner.cdp;
        let events = cdp.subscribe();
        let version = cdp.call(None, "Browser.getVersion", json!({})).await?;
        let ua = version
            .get("userAgent")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let shared = Arc::new(Shared {
            cdp: cdp.clone(),
            services: self.inner.services.clone(),
            headless: self.inner.headless,
            font_platform: FontPlatform::from_user_agent(ua),
            browser_version: version
                .get("product")
                .and_then(Value::as_str)
                .and_then(|p| p.split('/').nth(1))
                .unwrap_or_default()
                .to_owned(),
            launch_args: self.inner.launch_args.clone(),
            launch_proxy: self.inner.launch_proxy.clone(),
            artifacts: self.inner.artifacts.clone(),
            contexts: Mutex::new(HashMap::new()),
            downloads_dir: Mutex::new(None),
            downloads: Mutex::new(HashMap::new()),
            browser: Mutex::new(Arc::downgrade(&self.inner)),
            selectors: Mutex::new(Default::default()),
        });
        *self.inner.shared.lock().unwrap() = Some(shared.clone());
        // The default context must exist before targets attach.
        let ctx = match default_context {
            Some(o) => Some(BrowserContext::register(shared.clone(), o, String::new()).await?),
            None => None,
        };
        let weak = Arc::downgrade(&self.inner);
        let loop_shared = Arc::downgrade(&shared);
        tokio::spawn(async move {
            target_loop(loop_shared, events).await;
            // The connection is gone: Playwright's `disconnected`. During
            // `close()`, that emits it itself, after removing the profile.
            if let Some(inner) = weak.upgrade().filter(|i| !i.closed.load(Ordering::SeqCst)) {
                inner.emit_disconnected();
            }
        });
        cdp.call(
            None,
            "Target.setAutoAttach",
            json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
        )
        .await?;
        Ok(ctx)
    }

    fn shared(&self) -> Result<Arc<Shared>> {
        if !self.is_connected() {
            return Err(Error::TargetClosed("browser has been closed".into()));
        }
        self.inner
            .shared
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Error::TargetClosed("browser is not connected".into()))
    }

    /// Creates an isolated context (Playwright's `browser.newContext`).
    pub async fn new_context(&self, options: ContextOptions) -> Result<BrowserContext> {
        let ctx = BrowserContext::create(self.shared()?, options).await?;
        self.inner.emit(BrowserEvent::Context(ctx.clone()));
        Ok(ctx)
    }

    /// `browser.on(...)`: this browser's events from now on.
    pub fn events(&self) -> impl futures::Stream<Item = BrowserEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// A page in a fresh context of its own; closing the page closes the context.
    pub async fn new_page(&self) -> Result<Page> {
        self.new_page_with(ContextOptions::default()).await
    }

    /// `browser.newPage(options)`: a page in a fresh context with `options`;
    /// closing the page closes the context.
    pub async fn new_page_with(&self, options: ContextOptions) -> Result<Page> {
        let ctx = self.new_context(options).await?;
        let page = ctx.new_page().await?;
        page.inner.owns_context.store(true, Ordering::SeqCst);
        Ok(page)
    }

    /// Contexts created through this browser that are still open.
    pub fn contexts(&self) -> Vec<BrowserContext> {
        let Ok(shared) = self.shared() else {
            return Vec::new();
        };
        let contexts = shared.contexts.lock().unwrap();
        contexts
            .values()
            .map(|inner| BrowserContext {
                inner: inner.clone(),
            })
            .collect()
    }

    /// The browser's product string, e.g. `HeadlessChrome/157.0.8103.78`.
    pub async fn version(&self) -> Result<String> {
        let v = self
            .inner
            .cdp
            .call(None, "Browser.getVersion", json!({}))
            .await?;
        Ok(v.get("product")
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    /// The temporary profile directory of this launch.
    pub fn user_data_dir(&self) -> Option<PathBuf> {
        self.inner.user_data_dir.clone()
    }

    /// The executable that was launched.
    pub fn executable_path(&self) -> Option<PathBuf> {
        self.inner.executable.clone()
    }

    pub fn is_connected(&self) -> bool {
        !self.inner.closed.load(Ordering::SeqCst)
    }

    /// Raw protocol access to the browser target.
    pub fn cdp(&self) -> Arc<dyn CdpSession> {
        self.inner.cdp.clone()
    }

    pub(crate) fn clock(&self) -> Arc<dyn crate::ports::Clock> {
        self.inner.services.clock.clone()
    }

    /// The pid of the browser process this library launched (`None` when
    /// connected, or once closed).
    pub fn process_id(&self) -> Option<u32> {
        self.inner.process.lock().unwrap().as_ref()?.pid()
    }

    /// Closes the browser, ends the process and deletes the temporary profile.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.unbind().await;
        let live: Vec<_> = self
            .inner
            .shared
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.contexts.lock().unwrap().values().cloned().collect())
            .unwrap_or_default();
        for inner in live {
            let _ = BrowserContext { inner }.flush_har().await;
        }
        let mut context_ids: Vec<String> = Vec::new();
        let shared = self.inner.shared.lock().unwrap().take();
        if let Some(shared) = shared {
            let contexts: Vec<_> = shared
                .contexts
                .lock()
                .unwrap()
                .drain()
                .map(|(_, c)| c)
                .collect();
            for c in contexts {
                if !c.id.is_empty() {
                    context_ids.push(c.id.clone());
                }
                c.closed.store(true, Ordering::SeqCst);
                let pages = std::mem::take(&mut *c.pages.lock().unwrap());
                for p in pages {
                    p.mark_closed();
                }
                c.emit(crate::application::context::ContextEvent::Close);
            }
            shared.close_downloads();
            if let Some(dir) = shared.downloads_dir.lock().unwrap().take() {
                let _ = self.inner.services.fs.remove_dir_all(&dir);
            }
        }
        let process = self.inner.process.lock().unwrap().take();
        if process.is_none() {
            // Connected, not launched: like Playwright, drop the contexts this
            // client created and disconnect; the browser keeps running.
            for id in context_ids {
                let _ = self
                    .inner
                    .cdp
                    .call(
                        None,
                        "Target.disposeBrowserContext",
                        json!({"browserContextId": id}),
                    )
                    .await;
            }
            self.inner.cdp.disconnect();
            self.inner.emit_disconnected();
            return Ok(());
        }
        let _ = self.inner.cdp.call(None, "Browser.close", json!({})).await;
        if let Some(process) = process {
            process.shutdown().await;
        }
        if let Some(dir) = self
            .inner
            .user_data_dir
            .as_ref()
            .filter(|_| self.inner.remove_profile)
        {
            let _ = self.inner.services.fs.remove_dir_all(dir);
        }
        self.inner.emit_disconnected();
        Ok(())
    }
}

/// Resumes auto-attached targets; initialises pages of our contexts.
async fn target_loop(shared: Weak<Shared>, mut events: EventStream) {
    while let Some(event) = events.next().await {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        if event.session_id.is_none() && event.method.starts_with("Browser.download") {
            shared.handle_download_event(&event.method, &event.params);
            continue;
        }
        if event.session_id.is_none() && event.method == "Target.detachedFromTarget" {
            if let Some(sid) = event.params.get("sessionId").and_then(Value::as_str) {
                let contexts: Vec<_> = shared.contexts.lock().unwrap().values().cloned().collect();
                for c in contexts {
                    c.detach_service_worker(sid);
                }
            }
            continue;
        }
        if event.method != "Target.attachedToTarget" {
            continue;
        }
        let p = &event.params;
        let Some(session_id) = p
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let info = &p["targetInfo"];
        let kind = info.get("type").and_then(Value::as_str).unwrap_or_default();
        let target_id = info
            .get("targetId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let opener = info
            .get("openerId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let context = {
            let contexts = shared.contexts.lock().unwrap();
            info.get("browserContextId")
                .and_then(Value::as_str)
                .and_then(|id| contexts.get(id).cloned())
                // Targets of the default context (persistent launches).
                .or_else(|| contexts.get("").cloned())
        };
        match context {
            Some(context) if kind == "page" && event.session_id.is_none() => {
                let shared = shared.clone();
                tokio::spawn(async move {
                    match Page::initialize(shared, &context, session_id, target_id.clone(), opener)
                        .await
                    {
                        Ok(page) => {
                            context.har_track_page(&page);
                            context.trace_page(&page);
                            context.add_page(page)
                        }
                        Err(e) => context.page_failed(&target_id, &e),
                    }
                });
            }
            Some(context) if kind == "service_worker" && event.session_id.is_none() => {
                let url = info
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let worker = crate::application::worker::Worker::attach(
                    shared.clone(),
                    None,
                    session_id,
                    url,
                );
                context.add_service_worker(worker);
            }
            // Out-of-process iframes are attached by their page's event loop.
            _ if kind == "iframe" && event.session_id.is_some() => {}
            _ if kind == "worker" && event.session_id.is_some() => {
                let page = event
                    .session_id
                    .as_deref()
                    .and_then(|s| shared.page_by_session(s));
                match page {
                    Some(page) => {
                        let url = info
                            .get("url")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        page.attach_worker(session_id, url);
                    }
                    None => {
                        let fut = shared.cdp.call(
                            Some(&session_id),
                            "Runtime.runIfWaitingForDebugger",
                            json!({}),
                        );
                        tokio::spawn(fut);
                    }
                }
            }
            _ => {
                let fut = shared.cdp.call(
                    Some(&session_id),
                    "Runtime.runIfWaitingForDebugger",
                    json!({}),
                );
                tokio::spawn(fut);
            }
        }
    }
}

impl Browser {
    pub(crate) fn shared_ref(&self) -> Option<Arc<Shared>> {
        self.inner.shared.lock().unwrap().clone()
    }
}

impl Shared {
    pub(crate) fn browser(&self) -> Option<Browser> {
        self.browser
            .lock()
            .unwrap()
            .upgrade()
            .map(|inner| Browser { inner })
    }
}

impl Browser {
    pub(crate) fn tracing_session(&self) -> &Mutex<Option<Option<String>>> {
        &self.inner.tracing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_launches_over_the_devtools_port() {
        assert_eq!(effective_channel(Channel::Pipe, false), Channel::Pipe);
        assert_eq!(
            effective_channel(Channel::WebSocket, false),
            Channel::WebSocket
        );
        assert_eq!(effective_channel(Channel::Pipe, true), Channel::WebSocket);
        assert_eq!(
            effective_channel(Channel::WebSocket, true),
            Channel::WebSocket
        );
    }
    use crate::ports::FileSystem;
    use crate::testing::clock::TestClock;
    use crate::testing::memory::{MapEnvironment, MemoryFileSystem};
    use crate::testing::process::FakeLauncher;

    const SHELL: &str = "/opt/pw-browsers/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell";

    fn services(launcher: &FakeLauncher) -> (Services, Arc<MemoryFileSystem>) {
        let fs = Arc::new(MemoryFileSystem::with_files([SHELL]));
        let s = Services {
            fs: fs.clone(),
            env: Arc::new(MapEnvironment::linux("/home/u")),
            launcher: Arc::new(launcher.clone()),
            clock: Arc::new(TestClock),
            video: Arc::new(crate::testing::memory::NullVideo),
            http: Arc::new(crate::testing::memory::NoHttp),
            bridge: Arc::new(crate::testing::memory::NoBridge),
            signals: Arc::new(crate::testing::memory::NoSignals),
            adb: Arc::new(crate::testing::memory::NoAdb),
        };
        (s, fs)
    }

    #[tokio::test]
    async fn launch_runs_the_discovered_browser_with_the_preset_and_a_fresh_profile() {
        let launcher = FakeLauncher::default();
        let (s, fs) = services(&launcher);
        let browser = Browser::launch(
            s,
            LaunchOptions {
                args: vec!["--lang=de".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cmd = launcher.commands.lock().unwrap()[0].clone();
        assert_eq!(cmd.program, PathBuf::from(SHELL));
        let dir = browser.user_data_dir().unwrap();
        assert!(dir.to_string_lossy().contains("chromiumoxide-profile-"));
        assert!(fs.is_dir(&dir));
        assert_eq!(cmd.user_data_dir, dir);
        assert!(cmd.args.contains(&"--headless".to_owned()));
        assert!(cmd.args.contains(&"--lang=de".to_owned()));
        assert!(
            cmd.args
                .contains(&format!("--user-data-dir={}", dir.display()))
        );
        assert_eq!(cmd.channel, Channel::Pipe);
        assert_eq!(browser.executable_path(), Some(PathBuf::from(SHELL)));
    }

    #[tokio::test]
    async fn every_launch_gets_its_own_profile() {
        let launcher = FakeLauncher::default();
        let (s, _) = services(&launcher);
        let a = Browser::launch(s.clone(), LaunchOptions::default())
            .await
            .unwrap();
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        assert_ne!(a.user_data_dir(), b.user_data_dir());
    }

    #[tokio::test]
    async fn connect_handshake_matches_playwright() {
        let launcher = FakeLauncher::default();
        let (s, _) = services(&launcher);
        let _b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        let calls = launcher.cdp.calls();
        assert_eq!(calls[0].method, "Browser.getVersion");
        assert_eq!(calls[1].method, "Target.setAutoAttach");
        assert_eq!(
            calls[1].params,
            json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true})
        );
        assert_eq!(calls[1].session_id, None);
    }

    #[tokio::test]
    async fn version_reports_the_product() {
        let launcher = FakeLauncher::default();
        launcher.cdp.on_call(|_, c| {
            Ok(if c.method == "Browser.getVersion" {
                json!({"product": "HeadlessChrome/157.0"})
            } else {
                json!({})
            })
        });
        let (s, _) = services(&launcher);
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        assert_eq!(b.version().await.unwrap(), "HeadlessChrome/157.0");
    }

    #[tokio::test]
    async fn close_sends_browser_close_stops_the_process_and_removes_the_profile() {
        let launcher = FakeLauncher::default();
        let (s, fs) = services(&launcher);
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        let dir = b.user_data_dir().unwrap();
        b.close().await.unwrap();
        assert!(launcher.cdp.methods().contains(&"Browser.close".to_owned()));
        assert!(launcher.shut_down.load(Ordering::SeqCst));
        assert!(!fs.is_dir(&dir));
        assert!(!b.is_connected());
        b.close().await.unwrap(); // idempotent
    }

    #[tokio::test]
    async fn headful_and_full_chrome_use_new_headless_rules() {
        let launcher = FakeLauncher::default();
        let (mut s, _) = services(&launcher);
        s.fs = Arc::new(MemoryFileSystem::with_files(["/usr/bin/google-chrome"]));
        let opts = LaunchOptions {
            executable_path: Some("/usr/bin/google-chrome".into()),
            ..Default::default()
        };
        Browser::launch(s, opts).await.unwrap();
        let cmd = launcher.commands.lock().unwrap()[0].clone();
        assert!(cmd.args.contains(&"--headless=new".to_owned()));
    }
}
