//! Browser contexts: isolated profiles with their own emulation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::page::{DEFAULT_TIMEOUT, Page};
use crate::application::shared::Shared;
use crate::application::wait::wait_until;
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};

pub(crate) struct ContextInner {
    pub(crate) id: String,
    pub(crate) options: Arc<ContextOptions>,
    pub(crate) shared: Arc<Shared>,
    pub(crate) pages: Mutex<Vec<Page>>,
    /// Files of this context's downloads (deleted when it closes).
    pub(crate) downloaded: Mutex<Vec<std::path::PathBuf>>,
    /// `context.clock` has installed its script.
    pub(crate) clock_installed: std::sync::atomic::AtomicBool,
    pub(crate) debugger: Mutex<crate::application::debugger::DebuggerState>,
    pub(crate) init_errors: Mutex<HashMap<String, String>>,
    pub(crate) init_scripts: Mutex<Vec<String>>,
    pub(crate) bindings: Mutex<HashMap<String, crate::application::scripts::BindingFn>>,
    pub(crate) changed: watch::Sender<u64>,
    pub(crate) closed: AtomicBool,
    pub(crate) routes: crate::application::route::Routes,
    pub(crate) runtime: Mutex<crate::application::storage::RuntimeSettings>,
    /// Origins visited by the context's frames (for storage state).
    pub(crate) origins: crate::application::storage::Origins,
    pub(crate) subscribers: Mutex<Vec<futures::channel::mpsc::UnboundedSender<ContextEvent>>>,
    pub(crate) ws_routes: crate::application::websocket_route::WsRoutes,
    pub(crate) ws_routing: Mutex<crate::application::websocket_route::WsRouting>,
    pub(crate) service_workers: Mutex<Vec<crate::application::worker::Worker>>,
    pub(crate) credentials: crate::application::credentials::CredentialsState,
    pub(crate) har: crate::application::har::HarState,
    pub(crate) tracing: crate::application::tracing::TracingState,
    pub(crate) default_timeout: Mutex<Option<std::time::Duration>>,
    pub(crate) navigation_timeout: Mutex<Option<std::time::Duration>>,
    /// The browser of a persistent context (closed with it).
    pub(crate) owner: Mutex<Option<crate::application::browser::Browser>>,
}

/// Something that happened in a context (Playwright's `context.on(...)`).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ContextEvent {
    /// A page was created (`context.on('page')`), including popups.
    Page(Page),
    /// A service worker started (`context.on('serviceworker')`).
    ServiceWorker(crate::application::worker::Worker),
    /// The context was closed.
    Close,
    /// A request of any page or service worker (`context.on('request')`).
    Request(crate::application::network::Request),
    /// `context.on('response')`.
    Response(crate::application::network::Response),
    /// `context.on('requestfinished')`.
    RequestFinished(crate::application::network::Request),
    /// `context.on('requestfailed')`.
    RequestFailed(crate::application::network::Request),
    /// A console message of any page (`context.on('console')`).
    Console(crate::application::events::ConsoleMessage),
    /// An uncaught page error (`context.on('weberror')`).
    WebError(WebError),
    /// A dialog of any page (`context.on('dialog')`).
    Dialog(crate::application::events::Dialog),
    /// `context.on('dialogclosed')`.
    DialogClosed(crate::application::events::Dialog),
    /// A download of any page (`context.on('download')`).
    Download(crate::application::download::Download),
    /// `context.on('frameattached')`.
    FrameAttached(crate::application::frame::Frame),
    /// `context.on('framedetached')`.
    FrameDetached(crate::application::frame::Frame),
    /// `context.on('framenavigated')`.
    FrameNavigated(crate::application::frame::Frame),
    /// A page fired `load` (`context.on('pageload')`).
    PageLoad(Page),
    /// A page closed (`context.on('pageclose')`).
    PageClose(Page),
    /// `context.on('backgroundpage')`: never fires in Chromium (Playwright
    /// deprecates background pages; see `backgroundPages()`).
    BackgroundPage(Page),
}

/// Playwright's `WebError`: an uncaught error and its page.
#[derive(Debug, Clone)]
pub struct WebError {
    pub page: Option<Page>,
    pub error: crate::domain::console::PageError,
}

impl ContextInner {
    pub(crate) fn add_service_worker(self: &Arc<Self>, worker: crate::application::worker::Worker) {
        worker.set_owner(self);
        self.service_workers.lock().unwrap().push(worker.clone());
        self.emit(ContextEvent::ServiceWorker(worker));
    }

    pub(crate) fn detach_service_worker(&self, session_id: &str) {
        let mut list = self.service_workers.lock().unwrap();
        list.retain(|w| {
            if w.session_id() == session_id {
                w.close();
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn add_page(&self, page: Page) {
        self.pages.lock().unwrap().push(page.clone());
        self.changed.send_modify(|v| *v += 1);
        self.emit(ContextEvent::Page(page.clone()));
        if let Some(opener) = page.opener() {
            opener.emit(crate::application::events::PageEvent::Popup(page));
        }
    }

    pub(crate) fn emit(&self, event: ContextEvent) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }

    pub(crate) fn page_failed(&self, target_id: &str, error: &Error) {
        self.init_errors
            .lock()
            .unwrap()
            .insert(target_id.to_owned(), error.to_string());
        self.changed.send_modify(|v| *v += 1);
    }

    pub(crate) fn forget_page(&self, target_id: &str) {
        self.pages
            .lock()
            .unwrap()
            .retain(|p| p.target_id() != target_id);
        self.changed.send_modify(|v| *v += 1);
    }
}

/// An isolated browser profile (Playwright's `BrowserContext`). Cheap to clone.
#[derive(Clone)]
pub struct BrowserContext {
    pub(crate) inner: Arc<ContextInner>,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("id", &self.inner.id)
            .finish()
    }
}

impl BrowserContext {
    /// `Target.createBrowserContext` and registration.
    pub(crate) async fn create(
        shared: Arc<Shared>,
        options: ContextOptions,
    ) -> Result<BrowserContext> {
        options.validate()?;
        let r = shared
            .cdp
            .call(
                None,
                "Target.createBrowserContext",
                match &options.proxy {
                    Some(p) => json!({"disposeOnDetach": true, "proxyServer": p.server,
                        "proxyBypassList": crate::domain::launch::context_proxy_bypass_list(p)}),
                    None => json!({"disposeOnDetach": true}),
                },
            )
            .await?;
        let id = r
            .get("browserContextId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Serde("createBrowserContext returned no id".into()))?
            .to_owned();
        Self::register(shared, options, id).await
    }

    /// Registers a context (`id` empty: the browser's default context).
    pub(crate) async fn register(
        shared: Arc<Shared>,
        options: ContextOptions,
        id: String,
    ) -> Result<BrowserContext> {
        let (changed, _) = watch::channel(0);
        let har_state = crate::application::har::recording(&options.record_har);
        let inner = Arc::new(ContextInner {
            id: id.clone(),
            options: Arc::new(options),
            shared: shared.clone(),
            pages: Mutex::new(Vec::new()),
            downloaded: Mutex::new(Vec::new()),
            init_errors: Mutex::new(HashMap::new()),
            init_scripts: Mutex::new(Vec::new()),
            clock_installed: std::sync::atomic::AtomicBool::new(false),
            debugger: Mutex::new(Default::default()),
            bindings: Mutex::new(HashMap::new()),
            changed,
            closed: AtomicBool::new(false),
            routes: Mutex::new(Vec::new()),
            runtime: Mutex::new(Default::default()),
            origins: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            ws_routes: Mutex::new(Vec::new()),
            ws_routing: Mutex::new(Default::default()),
            service_workers: Mutex::new(Vec::new()),
            credentials: Mutex::new(Default::default()),
            har: har_state,
            tracing: Mutex::new(None),
            default_timeout: Mutex::new(None),
            navigation_timeout: Mutex::new(None),
            owner: Mutex::new(None),
        });
        shared.contexts.lock().unwrap().insert(id, inner.clone());
        if inner.options.service_workers == crate::domain::emulation::ServiceWorkers::Block {
            inner.init_scripts.lock().unwrap().push(
                "if (navigator.serviceWorker) navigator.serviceWorker.register = async () => { console.warn('Service Worker registration blocked by chromiumoxide'); };"
                    .to_owned(),
            );
        }
        let context = BrowserContext { inner };
        let accept = context.inner.options.accept_downloads;
        let mut params = json!({
            "behavior": if accept { "allowAndName" } else { "deny" },
            "eventsEnabled": true,
        });
        if !context.inner.id.is_empty() {
            params["browserContextId"] = json!(context.inner.id);
        }
        if let Ok(dir) = shared.downloads_dir() {
            params["downloadPath"] = json!(dir.to_string_lossy());
        }
        shared
            .cdp
            .call(None, "Browser.setDownloadBehavior", params)
            .await?;
        if !context.inner.options.permissions.is_empty() {
            let names: Vec<&str> = context
                .inner
                .options
                .permissions
                .iter()
                .map(String::as_str)
                .collect();
            context.grant_permissions(&names, None).await?;
        }
        if let Some(state) = context.inner.options.storage_state.clone() {
            context.restore_storage_state(&state).await?;
        }
        Ok(context)
    }

    /// The protocol id of this context.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// The options this context was created with.
    pub fn options(&self) -> &ContextOptions {
        &self.inner.options
    }

    /// Opens a new page in this context.
    pub async fn new_page(&self) -> Result<Page> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("context has been closed".into()));
        }
        let shared = &self.inner.shared;
        let r = shared
            .cdp
            .call(
                None,
                "Target.createTarget",
                if self.inner.id.is_empty() {
                    json!({"url": "about:blank"})
                } else {
                    json!({"url": "about:blank", "browserContextId": self.inner.id})
                },
            )
            .await?;
        let target_id = r
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Serde("createTarget returned no targetId".into()))?
            .to_owned();
        let inner = &self.inner;
        wait_until(
            &*shared.services.clock,
            &inner.changed,
            DEFAULT_TIMEOUT,
            "creating a page",
            || {
                if let Some(e) = inner.init_errors.lock().unwrap().remove(&target_id) {
                    return Some(Err(Error::TargetClosed(format!(
                        "page failed to initialise: {e}"
                    ))));
                }
                inner
                    .pages
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.target_id() == target_id)
                    .cloned()
                    .map(Ok)
            },
        )
        .await
    }

    /// Every event from now on (`context.on(...)`).
    pub fn events(&self) -> impl futures::Stream<Item = ContextEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    /// `context.waitForEvent('page')`.
    pub fn wait_for_page(&self) -> PageWaiter<'_> {
        PageWaiter {
            context: self,
            timeout: None,
        }
    }

    /// `context.serviceWorkers()`: running service workers.
    pub fn service_workers(&self) -> Vec<crate::application::worker::Worker> {
        self.inner.service_workers.lock().unwrap().clone()
    }

    /// Open pages.
    pub fn pages(&self) -> Vec<Page> {
        self.inner.pages.lock().unwrap().clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Closes all pages and disposes the context.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.flush_har().await;
        // A persistent (default) context closes its browser.
        let owner = self.inner.owner.lock().unwrap().take();
        if let Some(browser) = owner {
            return browser.close().await;
        }
        let shared = &self.inner.shared;
        let _ = shared
            .cdp
            .call(
                None,
                "Target.disposeBrowserContext",
                json!({"browserContextId": self.inner.id}),
            )
            .await;
        let pages = std::mem::take(&mut *self.inner.pages.lock().unwrap());
        for p in pages {
            p.mark_closed();
        }
        shared.contexts.lock().unwrap().remove(&self.inner.id);
        // Like Playwright, downloads go with their context.
        for path in std::mem::take(&mut *self.inner.downloaded.lock().unwrap()) {
            let _ = shared.services.fs.remove_file(&path);
        }
        self.inner.changed.send_modify(|v| *v += 1);
        self.inner.emit(ContextEvent::Close);
        Ok(())
    }
}

/// A pending `context.waitForEvent('page')`.
#[must_use = "does nothing until awaited"]
pub struct PageWaiter<'a> {
    context: &'a BrowserContext,
    timeout: Option<std::time::Duration>,
}

impl<'a> PageWaiter<'a> {
    pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Subscribes, runs `action`, then waits for the new page.
    pub async fn run<A, R>(self, action: A) -> Result<Page>
    where
        A: std::future::IntoFuture<Output = Result<R>>,
    {
        let events = self.context.events();
        action.await?;
        self.wait_on(events).await
    }

    async fn wait_on(
        self,
        mut events: impl futures::Stream<Item = ContextEvent> + Unpin,
    ) -> Result<Page> {
        use futures::StreamExt;
        let timeout = self.timeout.unwrap_or(DEFAULT_TIMEOUT);
        let find = async move {
            while let Some(e) = events.next().await {
                match e {
                    ContextEvent::Page(p) => return Ok(p),
                    ContextEvent::Close => break,
                    _ => {}
                }
            }
            Err(Error::TargetClosed(
                "context closed while waiting for a page".into(),
            ))
        };
        crate::application::wait::with_timeout(
            &*self.context.inner.shared.services.clock,
            timeout,
            "waiting for event \"page\"",
            find,
        )
        .await
    }
}

impl<'a> std::future::IntoFuture for PageWaiter<'a> {
    type Output = Result<Page>;
    type IntoFuture = futures::future::BoxFuture<'a, Result<Page>>;
    fn into_future(self) -> Self::IntoFuture {
        use futures::FutureExt;
        async move {
            let events = self.context.events();
            self.wait_on(events).await
        }
        .boxed()
    }
}
