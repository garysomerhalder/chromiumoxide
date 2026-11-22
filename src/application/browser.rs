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
    /// Extra environment variables for the browser process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the browser to start (default 180 s, like Playwright).
    pub timeout: Duration,
    /// Pipe (default) or WebSocket.
    pub channel: Channel,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            headless: true,
            args: Vec::new(),
            ignore_default_args: false,
            env: Vec::new(),
            timeout: Duration::from_secs(180),
            channel: Channel::Pipe,
        }
    }
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
    /// `LaunchOptions::args` (None when connected).
    launch_args: Option<Vec<String>>,
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

impl Browser {
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
            user_data_dir: user_data_dir.clone(),
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
                launch_args: Some(options.args.clone()),
            }),
        };
        let default_options = persistent.map(|(_, o)| o);
        match browser.handshake(default_options).await {
            Ok(ctx) => Ok((browser, ctx)),
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
                launch_args: None,
            }),
        };
        browser.handshake(None).await?;
        Ok(browser)
    }

    /// Playwright's `browser.bind()`: lets other clients
    /// ([`Chromium::connect`](crate::Chromium::connect)) share this browser
    /// over a local WebSocket, whatever channel it was launched with. Their
    /// endpoint speaks CDP. Binding again returns the same endpoint.
    pub async fn bind(&self) -> Result<String> {
        if let Some(b) = self.inner.bridge.lock().unwrap().as_ref() {
            return Ok(b.endpoint());
        }
        let bound = self
            .inner
            .services
            .bridge
            .serve(self.inner.cdp.clone())
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
        Ok(())
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
        tokio::spawn(target_loop(Arc::downgrade(&shared), events));
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
        BrowserContext::create(self.shared()?, options).await
    }

    /// A page in a fresh context of its own; closing the page closes the context.
    pub async fn new_page(&self) -> Result<Page> {
        let ctx = self.new_context(ContextOptions::default()).await?;
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

    /// Closes the browser, ends the process and deletes the temporary profile.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
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
