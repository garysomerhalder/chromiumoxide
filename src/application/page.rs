//! A page (tab): its frame model, event loop and the Playwright-like API.

use std::collections::{HashMap, HashSet};
use std::future::IntoFuture;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::{FutureExt, StreamExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::context::ContextInner;
use crate::application::js;
use crate::application::page_init::{InitContext, page_init_commands, viewport_commands};
use crate::application::shared::Shared;
use crate::application::wait::{wait_until, with_timeout};
use crate::domain::emulation::{ContextOptions, Viewport};
use crate::domain::error::{Error, Result};
use crate::domain::frame::{Effect, FrameTree, LifecycleEvent, NETWORK_IDLE_MS, World};
use crate::ports::{CdpSession, EventStream, RawEvent};

/// The credentials for proxy challenges: the context's proxy, else the
/// browser's (`LaunchOptions::proxy`).
fn proxy_credentials(
    options: &crate::domain::emulation::ContextOptions,
    shared: &crate::application::shared::Shared,
) -> Option<(String, String)> {
    let p = options.proxy.as_ref().or(shared.launch_proxy.as_ref())?;
    match (&p.username, &p.password) {
        (Some(u), p) => Some((u.clone(), p.clone().unwrap_or_default())),
        _ => None,
    }
}

/// Name of the isolated world chromiumoxide uses for its helpers.
pub const UTILITY_WORLD: &str = "__chromiumoxide_utility_world__";

/// Playwright's default timeout for actions and navigations.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

pub use crate::application::network::Response;

#[derive(Default)]
pub(crate) struct PageState {
    pub(crate) tree: FrameTree,
    pub(crate) responses: HashMap<String, Response>,
    auth_attempts: HashSet<String>,
    pub(crate) documents_opened: u64,
    pub(crate) viewport: Option<Viewport>,
    /// Bumped on every main-frame navigation (new document or same document).
    pub(crate) main_navigations: u64,
    /// Whether the latest main-frame navigation stayed in the same document.
    pub(crate) last_navigation_same_document: bool,
    /// Per frame: navigations so far and whether the last stayed in the document.
    pub(crate) frame_navigations: HashMap<String, (u64, bool)>,
}

pub(crate) struct PageInner {
    pub(crate) shared: Arc<Shared>,
    pub(crate) context: Weak<ContextInner>,
    pub(crate) options: Arc<ContextOptions>,
    pub(crate) session_id: String,
    pub(crate) target_id: String,
    pub(crate) window_id: Option<i64>,
    pub(crate) state: Mutex<PageState>,
    pub(crate) changed: watch::Sender<u64>,
    pub(crate) closed: AtomicBool,
    /// Created by `browser.new_page()`: closing the page closes its context.
    pub(crate) owns_context: AtomicBool,
    pub(crate) default_timeout: Mutex<Duration>,
    pub(crate) bindings: Mutex<HashMap<String, crate::application::scripts::BindingFn>>,
    pub(crate) keyboard: tokio::sync::Mutex<crate::domain::input::Keyboard>,
    pub(crate) mouse: tokio::sync::Mutex<crate::domain::input::Mouse>,
    pub(crate) events: Mutex<crate::application::events::EventState>,
    pub(crate) network: Mutex<crate::application::network::NetworkState>,
    pub(crate) routes: crate::application::route::Routes,
    /// Whether `Fetch` interception is on (routes or HTTP credentials).
    pub(crate) intercepting: AtomicBool,
    /// `page.setExtraHTTPHeaders()`.
    pub(crate) extra_http_headers: Mutex<Option<std::collections::BTreeMap<String, String>>>,
    /// The target that opened this page (popups).
    pub(crate) opener_id: Option<String>,
    pub(crate) websockets: Mutex<HashMap<String, crate::application::websocket::WebSocket>>,
    pub(crate) workers: Mutex<Vec<crate::application::worker::Worker>>,
    /// Pending file chooser waits (interception is on while > 0).
    pub(crate) file_chooser_waiters: std::sync::atomic::AtomicI64,
    /// `route.fetch()` calls waiting for their response stage, by interception id.
    pub(crate) response_waiters: Mutex<HashMap<String, futures::channel::oneshot::Sender<Value>>>,
    pub(crate) ws_routes: crate::application::websocket_route::WsRoutes,
    pub(crate) video: Mutex<Option<crate::application::video::Video>>,
    pub(crate) screencast: Mutex<crate::application::screencast::ScreencastState>,
    /// The API calls running on this page (method, params), innermost last.
    pub(crate) calls: Mutex<Vec<(String, Value)>>,
    /// The data of an HTML5 drag in progress (`Input.dragIntercepted`).
    pub(crate) drag: Mutex<Option<Value>>,
    /// Sessions of out-of-process iframes.
    pub(crate) oopifs: Mutex<Vec<crate::application::oopif::Oopif>>,
    pub(crate) coverage: Mutex<crate::application::coverage::CoverageState>,
    /// Setup commands to replay on out-of-process iframes (see `oopif`).
    pub(crate) setup_log: Mutex<Vec<(String, Value)>>,
    /// `page.pickLocator()` is waiting for a pick.
    pub(crate) picking: AtomicBool,
    pub(crate) navigation_timeout: Mutex<Option<Duration>>,
    pub(crate) media: Mutex<crate::application::extras::EmulateMedia>,
    pub(crate) locator_handlers: Mutex<Vec<crate::application::extras::LocatorHandler>>,
    /// A locator handler is running (actions inside it skip the handlers).
    pub(crate) handling_locator: AtomicBool,
}

impl PartialEq for Page {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Eq for Page {}

/// A browser tab. Cheap to clone.
#[derive(Clone)]
pub struct Page {
    pub(crate) inner: Arc<PageInner>,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("target_id", &self.inner.target_id)
            .finish()
    }
}

impl Page {
    /// Initialises a freshly attached (paused) page target like Playwright does.
    pub(crate) async fn initialize(
        shared: Arc<Shared>,
        context: &Arc<ContextInner>,
        session_id: String,
        target_id: String,
        opener_id: Option<String>,
    ) -> Result<Page> {
        let cdp = shared.cdp.clone();
        let sid_owned = session_id.clone();
        let sid = Some(sid_owned.as_str());
        let events = cdp.subscribe();
        let options = context.options.clone();

        let window_id = if options.viewport.is_some() {
            cdp.call(sid, "Browser.getWindowForTarget", json!({}))
                .await
                .ok()
                .and_then(|r| r.get("windowId").and_then(Value::as_i64))
        } else {
            None
        };
        let frame_tree = cdp.call(sid, "Page.getFrameTree", json!({}));
        let plan = page_init_commands(&InitContext {
            options: &options,
            headless: shared.headless,
            font_platform: shared.font_platform,
            window_id,
            utility_world: UTILITY_WORLD,
            proxy_auth: proxy_credentials(&options, &shared).is_some(),
        });
        let commands: Vec<(&str, Value)> = plan
            .into_iter()
            .chain(crate::application::scripts::context_script_commands(
                context,
            ))
            .chain(crate::application::route::context_route_commands(context))
            .chain(crate::application::storage::runtime_commands(context))
            .collect();
        // Out-of-process iframes get the same setup when they attach.
        let setup: Vec<(String, Value)> = commands
            .iter()
            .filter(|(m, _)| crate::application::oopif::BROADCAST.contains(m))
            .map(|(m, p)| ((*m).to_owned(), p.clone()))
            .collect();
        let mut calls: Vec<_> = commands
            .into_iter()
            .map(|(method, params)| cdp.call(sid, method, params))
            .collect();
        crate::application::credentials::setup_page(&cdp, &sid_owned, &target_id, context).await?;
        calls.push(cdp.call(sid, "Runtime.runIfWaitingForDebugger", json!({})));
        let tree = frame_tree.await?;

        let (changed, _) = watch::channel(0u64);
        let inner = Arc::new(PageInner {
            shared: shared.clone(),
            context: Arc::downgrade(context),
            options: options.clone(),
            session_id,
            target_id,
            window_id,
            state: Mutex::new(PageState::default()),
            changed,
            closed: AtomicBool::new(false),
            owns_context: AtomicBool::new(false),
            default_timeout: Mutex::new(
                context
                    .default_timeout
                    .lock()
                    .unwrap()
                    .unwrap_or(DEFAULT_TIMEOUT),
            ),
            bindings: Mutex::new(HashMap::new()),
            keyboard: tokio::sync::Mutex::new(crate::domain::input::Keyboard::new(
                shared.font_platform == crate::domain::emulation::FontPlatform::Mac,
            )),
            mouse: tokio::sync::Mutex::new(crate::domain::input::Mouse::default()),
            events: Mutex::new(Default::default()),
            network: Mutex::new(Default::default()),
            routes: Mutex::new(Vec::new()),
            extra_http_headers: Mutex::new(None),
            opener_id,
            websockets: Mutex::new(HashMap::new()),
            workers: Mutex::new(Vec::new()),
            file_chooser_waiters: std::sync::atomic::AtomicI64::new(0),
            response_waiters: Mutex::new(HashMap::new()),
            ws_routes: Mutex::new(Vec::new()),
            video: Mutex::new(None),
            screencast: Mutex::new(Default::default()),
            calls: Mutex::new(Vec::new()),
            drag: Mutex::new(None),
            oopifs: Mutex::new(Vec::new()),
            coverage: Mutex::new(Default::default()),
            setup_log: Mutex::new(setup),
            picking: AtomicBool::new(false),
            navigation_timeout: Mutex::new(*context.navigation_timeout.lock().unwrap()),
            media: Mutex::new(Default::default()),
            locator_handlers: Mutex::new(Vec::new()),
            handling_locator: AtomicBool::new(false),
            intercepting: AtomicBool::new(
                options.http_credentials.is_some() || !context.routes.lock().unwrap().is_empty(),
            ),
        });
        let page = Page { inner };
        let mut effects = Vec::new();
        {
            let mut st = page.inner.state.lock().unwrap();
            load_frame_tree(&mut st.tree, &tree["frameTree"], None, &mut effects);
        }
        page.apply_effects(effects);
        page.spawn_event_loop(events);
        page.start_video();

        for result in futures::future::join_all(calls).await {
            result?;
        }
        let frames: Vec<String> = page
            .inner
            .state
            .lock()
            .unwrap()
            .tree
            .frames()
            .map(|f| f.id.clone())
            .collect();
        for frame_id in frames {
            let _ = cdp
                .call(
                    sid,
                    "Page.createIsolatedWorld",
                    json!({"frameId": frame_id, "grantUniveralAccess": true, "worldName": UTILITY_WORLD}),
                )
                .await;
        }
        page.main_context(World::Main, DEFAULT_TIMEOUT).await?;
        Ok(page)
    }

    fn spawn_event_loop(&self, mut events: EventStream) {
        let weak = Arc::downgrade(&self.inner);
        let session = self.inner.session_id.clone();
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let Some(inner) = weak.upgrade() else { break };
                let page = Page { inner };
                if event.session_id.as_deref() == Some(session.as_str()) {
                    page.handle_event(&event);
                } else if let Some(no) = event.session_id.as_deref().and_then(|s| page.oopif_no(s))
                {
                    if let Some(e) = page.oopif_event(no, &event) {
                        page.handle_event(&e);
                    }
                } else if event.session_id.is_none()
                    && event.method == "Target.detachedFromTarget"
                    && event.params.get("sessionId").and_then(Value::as_str)
                        == Some(session.as_str())
                {
                    page.mark_closed();
                    break;
                }
                if page.is_closed() {
                    break;
                }
            }
            if let Some(inner) = weak.upgrade() {
                Page { inner }.mark_closed();
            }
        });
    }

    fn handle_event(&self, e: &RawEvent) {
        if self.handle_page_event(e) {
            return;
        }
        if e.method.starts_with("Network.webSocket") {
            self.handle_websocket_event(e);
            return;
        }
        if e.method.starts_with("Network.") {
            self.handle_network_event(e);
        }
        if e.method == "Target.attachedToTarget"
            && e.params.pointer("/targetInfo/type").and_then(Value::as_str) == Some("iframe")
        {
            let session = e.params.get("sessionId").and_then(Value::as_str);
            let target = e
                .params
                .pointer("/targetInfo/targetId")
                .and_then(Value::as_str);
            if let (Some(session), Some(target)) = (session, target) {
                let page = self.clone();
                let (session, target) = (session.to_owned(), target.to_owned());
                tokio::spawn(async move { page.attach_oopif(session, target).await });
            }
            return;
        }
        if e.method == "Target.detachedFromTarget" {
            if let Some(sid) = e.params.get("sessionId").and_then(Value::as_str) {
                self.detach_worker(sid);
            }
            return;
        }
        let p = &e.params;
        let s = |v: &Value, k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut effects = Vec::new();
        let mut frame_events: Vec<crate::application::events::PageEvent> = Vec::new();
        {
            let mut st = self.inner.state.lock().unwrap();
            match e.method.as_str() {
                "Page.frameAttached" => {
                    let id = s(p, "frameId");
                    let known = st.tree.frame(&id).is_some();
                    effects = st
                        .tree
                        .attached(&id, p.get("parentFrameId").and_then(Value::as_str));
                    if !known {
                        frame_events.push(crate::application::events::PageEvent::FrameAttached(
                            self.frame_by_id(&id),
                        ));
                    }
                }
                "Page.frameNavigated" => {
                    let f = &p["frame"];
                    frame_events.push(crate::application::events::PageEvent::FrameNavigated(
                        self.frame_by_id(&s(f, "id")),
                    ));
                    let url = format!("{}{}", s(f, "url"), s(f, "urlFragment"));
                    if let Some(origin) = crate::application::storage::origin_of(&url)
                        && let Some(ctx) = self.inner.context.upgrade()
                    {
                        ctx.add_origin(origin);
                    }
                    if f.get("parentId").is_none() {
                        st.main_navigations += 1;
                        st.last_navigation_same_document = false;
                    }
                    let n = st.frame_navigations.entry(s(f, "id")).or_default();
                    *n = (n.0 + 1, false);
                    effects = st.tree.navigated(
                        &s(f, "id"),
                        f.get("parentId").and_then(Value::as_str),
                        &url,
                        &s(f, "name"),
                        &s(f, "loaderId"),
                    );
                }
                "Page.navigatedWithinDocument" => {
                    let frame_id = s(p, "frameId");
                    frame_events.push(crate::application::events::PageEvent::FrameNavigated(
                        self.frame_by_id(&frame_id),
                    ));
                    if st.tree.main_frame().is_some_and(|m| m.id == frame_id) {
                        st.main_navigations += 1;
                        st.last_navigation_same_document = true;
                    }
                    let n = st.frame_navigations.entry(frame_id.clone()).or_default();
                    *n = (n.0 + 1, true);
                    st.tree.navigated_within_document(&frame_id, &s(p, "url"));
                }
                "Page.frameDetached" => {
                    if p.get("reason").and_then(Value::as_str) != Some("swap") {
                        let id = s(p, "frameId");
                        if st.tree.frame(&id).is_some() {
                            frame_events.push(
                                crate::application::events::PageEvent::FrameDetached(
                                    self.frame_by_id(&id),
                                ),
                            );
                        }
                        st.tree.detached(&id);
                    }
                }
                "Page.lifecycleEvent" => {
                    st.tree
                        .lifecycle(&s(p, "frameId"), &s(p, "loaderId"), &s(p, "name"))
                }
                "Page.documentOpened" => {
                    effects = st.tree.clear_lifecycle(&s(&p["frame"], "id"));
                    st.documents_opened += 1;
                }
                "Runtime.executionContextCreated" => {
                    let c = &p["context"];
                    let aux = &c["auxData"];
                    let frame_id = s(aux, "frameId");
                    let id = c.get("id").and_then(Value::as_i64).unwrap_or_default();
                    if aux.get("isDefault").and_then(Value::as_bool) == Some(true) {
                        st.tree.context_created(&frame_id, World::Main, id);
                    } else if c.get("name").and_then(Value::as_str) == Some(UTILITY_WORLD) {
                        st.tree.context_created(&frame_id, World::Utility, id);
                    }
                }
                "Runtime.executionContextDestroyed" => {
                    if let Some(id) = p.get("executionContextId").and_then(Value::as_i64) {
                        st.tree.context_destroyed(id);
                    }
                }
                "Runtime.executionContextsCleared" => st.tree.contexts_cleared(),
                "Network.requestWillBeSent" => {
                    let url = p
                        .pointer("/request/url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let has_redirect = p.get("redirectResponse").is_some();
                    if !url.starts_with("data:") && !has_redirect {
                        let rid = s(p, "requestId");
                        let is_document = s(p, "type") == "Document" && rid == s(p, "loaderId");
                        effects = st.tree.request_started(&s(p, "frameId"), &rid, is_document);
                    }
                }
                "Network.loadingFinished" | "Network.loadingFailed" => {
                    effects = st.tree.request_finished(&s(p, "requestId"));
                }
                "Fetch.requestPaused" => {
                    drop(st);
                    self.on_request_paused(p);
                    return;
                }
                "Fetch.authRequired" => {
                    let request_id = s(p, "requestId");
                    let url = p
                        .pointer("/request/url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    // Playwright 1.65: proxy credentials answer only the
                    // proxy's challenges, site credentials only the site's.
                    let is_proxy =
                        p.pointer("/authChallenge/source").and_then(Value::as_str) == Some("Proxy");
                    let creds = if is_proxy {
                        proxy_credentials(&self.inner.options, &self.inner.shared)
                    } else {
                        self.http_credentials()
                            .filter(|c| match &c.origin {
                                None => true,
                                Some(o) => origin_of(url).eq_ignore_ascii_case(o),
                            })
                            .map(|c| (c.username, c.password))
                    };
                    let attempt = format!("{}{request_id}", if is_proxy { "proxy:" } else { "" });
                    let response = if st.auth_attempts.contains(&attempt) {
                        json!({"response": "CancelAuth"})
                    } else if let Some((username, password)) = creds {
                        st.auth_attempts.insert(attempt);
                        json!({"response": "ProvideCredentials", "username": username, "password": password})
                    } else {
                        json!({"response": "Default"})
                    };
                    let page = self.clone();
                    let params =
                        json!({"requestId": request_id, "authChallengeResponse": response});
                    tokio::spawn(async move {
                        let _ = page.send("Fetch.continueWithAuth", params).await;
                    });
                }
                "Runtime.bindingCalled" => {
                    drop(st);
                    self.on_binding_called(p);
                    return;
                }
                "Inspector.targetCrashed" => {
                    drop(st);
                    self.emit(crate::application::events::PageEvent::Crash);
                    self.mark_closed();
                    return;
                }
                _ => return,
            }
        }
        self.apply_effects(effects);
        self.bump();
        for e in frame_events {
            self.emit(e);
        }
    }

    pub(crate) fn apply_effects(&self, effects: Vec<Effect>) {
        for Effect::StartNetworkIdleTimer {
            frame_id,
            generation,
        } in effects
        {
            let weak = Arc::downgrade(&self.inner);
            let sleep = self
                .inner
                .shared
                .services
                .clock
                .sleep(Duration::from_millis(NETWORK_IDLE_MS));
            tokio::spawn(async move {
                sleep.await;
                if let Some(inner) = weak.upgrade() {
                    inner
                        .state
                        .lock()
                        .unwrap()
                        .tree
                        .network_idle_timer_fired(&frame_id, generation);
                    inner.changed.send_modify(|v| *v += 1);
                }
            });
        }
    }

    pub(crate) fn bump(&self) {
        self.inner.changed.send_modify(|v| *v += 1);
    }

    pub(crate) fn mark_closed(&self) {
        if !self.inner.closed.swap(true, Ordering::SeqCst) {
            self.bump();
            self.close_network();
            self.close_websockets();
            self.close_workers();
            self.emit(crate::application::events::PageEvent::Close);
            if let Some(ctx) = self.inner.context.upgrade() {
                ctx.forget_page(&self.inner.target_id);
            }
        }
    }

    pub(crate) fn cdp_session(&self) -> &dyn CdpSession {
        &*self.inner.shared.cdp
    }

    pub(crate) fn sid(&self) -> Option<&str> {
        Some(self.inner.session_id.as_str())
    }

    /// Sends a raw protocol command to this page's session.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.ensure_open()?;
        if crate::application::oopif::BROADCAST.contains(&method) {
            self.record_setup(method, &params);
            self.broadcast_setup(method, &params).await;
        }
        if self.inner.oopifs.lock().unwrap().is_empty() {
            return self.cdp_session().call(self.sid(), method, params).await;
        }
        let mut params = params;
        match self.route_session(method, &mut params) {
            0 => self.cdp_session().call(self.sid(), method, params).await,
            no => self.send_in_session(no, method, params).await,
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.is_closed() {
            Err(Error::TargetClosed("page has been closed".into()))
        } else {
            Ok(())
        }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// The target id (stable for the page's lifetime).
    pub fn target_id(&self) -> &str {
        &self.inner.target_id
    }

    /// Sets the default timeout for this page's waits (Playwright: 30 s).
    pub fn set_default_timeout(&self, timeout: Duration) {
        *self.inner.default_timeout.lock().unwrap() = timeout;
    }

    pub(crate) fn default_timeout(&self) -> Duration {
        *self.inner.default_timeout.lock().unwrap()
    }

    /// The main frame's current URL.
    pub fn url(&self) -> String {
        self.inner
            .state
            .lock()
            .unwrap()
            .tree
            .main_frame()
            .map(|f| f.url.clone())
            .unwrap_or_default()
    }

    pub(crate) fn main_frame_id(&self) -> Result<String> {
        self.inner
            .state
            .lock()
            .unwrap()
            .tree
            .main_frame()
            .map(|f| f.id.clone())
            .ok_or_else(|| Error::TargetClosed("page has no main frame".into()))
    }

    /// Waits for (and returns) the main frame's execution context of `world`.
    pub(crate) async fn main_context(&self, world: World, timeout: Duration) -> Result<i64> {
        self.frame_context(None, world, timeout).await
    }

    /// Waits for (and returns) a frame's execution context of `world`
    /// (`None`: the main frame).
    pub(crate) async fn frame_context(
        &self,
        frame: Option<&str>,
        world: World,
        timeout: Duration,
    ) -> Result<i64> {
        let inner = &self.inner;
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            timeout,
            "waiting for an execution context",
            || {
                if inner.closed.load(Ordering::SeqCst) {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                let id = match frame {
                    Some(id) => {
                        if st.tree.frame(id).is_none() {
                            return Some(Err(Error::TargetClosed(
                                "frame has been detached".into(),
                            )));
                        }
                        id.to_owned()
                    }
                    None => st.tree.main_frame()?.id.clone(),
                };
                st.tree.context(&id, world).map(Ok)
            },
        )
        .await
    }

    /// Evaluates in a world of the main frame, retrying across navigations.
    pub(crate) async fn evaluate_value(
        &self,
        world: World,
        source: &str,
        arg: Option<&Value>,
    ) -> Result<Value> {
        let timeout = self.default_timeout();
        if !self.is_expression(world, source, timeout).await? {
            return self.evaluate_script(world, source, timeout).await;
        }
        let args: Vec<Value> = arg.cloned().into_iter().collect();
        let declaration = js::function_declaration(source);
        self.call_function(world, &declaration, &args, timeout, "evaluating")
            .await
    }

    /// Whether `source` parses as a single expression (or function), checked
    /// with `Runtime.compileScript` without running it.
    pub(crate) async fn is_expression(
        &self,
        world: World,
        source: &str,
        timeout: Duration,
    ) -> Result<bool> {
        self.is_expression_in(None, world, source, timeout).await
    }

    /// [`Page::is_expression`] in a given frame.
    pub(crate) async fn is_expression_in(
        &self,
        frame: Option<&str>,
        world: World,
        source: &str,
        timeout: Duration,
    ) -> Result<bool> {
        let context_id = self.frame_context(frame, world, timeout).await?;
        let r = self
            .send(
                "Runtime.compileScript",
                json!({"expression": format!("({}\n)", source.trim().trim_end_matches(';')),
                       "sourceURL": "", "persistScript": false, "executionContextId": context_id}),
            )
            .await?;
        Ok(r.get("exceptionDetails").is_none())
    }

    /// Playwright's semantics for a non-expression string: run it as a script
    /// (statements allowed), return the completion value, await a promise.
    async fn evaluate_script(
        &self,
        world: World,
        source: &str,
        timeout: Duration,
    ) -> Result<Value> {
        self.evaluate_script_in(None, world, source, timeout).await
    }

    /// [`Page::evaluate_script`] in a given frame.
    pub(crate) async fn evaluate_script_in(
        &self,
        frame: Option<&str>,
        world: World,
        source: &str,
        timeout: Duration,
    ) -> Result<Value> {
        let attempt = async {
            loop {
                let context_id = self.frame_context(frame, world, timeout).await?;
                let params = json!({"expression": source, "contextId": context_id, "returnByValue": true,
                                    "awaitPromise": true, "userGesture": true});
                match self.send("Runtime.evaluate", params).await {
                    Ok(response) => match js::result_value(&response) {
                        Err(e) if js::is_context_lost(&e) => continue,
                        other => return other,
                    },
                    Err(e) if js::is_context_lost(&e) => continue,
                    Err(e) => return Err(e),
                }
            }
        };
        with_timeout(
            &*self.inner.shared.services.clock,
            timeout,
            "evaluating",
            attempt,
        )
        .await
    }

    /// `Runtime.callFunctionOn` in the main frame's `world`, by value, retrying
    /// when a navigation replaces the execution context.
    pub(crate) async fn call_function(
        &self,
        world: World,
        declaration: &str,
        args: &[Value],
        timeout: Duration,
        action: &str,
    ) -> Result<Value> {
        self.call_function_in(None, world, declaration, args, timeout, action)
            .await
    }

    /// [`Page::call_function`] in a given frame (`None`: the main frame).
    pub(crate) async fn call_function_in(
        &self,
        frame: Option<&str>,
        world: World,
        declaration: &str,
        args: &[Value],
        timeout: Duration,
        action: &str,
    ) -> Result<Value> {
        self.ensure_open()?;
        let attempt = async {
            loop {
                let context_id = self.frame_context(frame, world, timeout).await?;
                let params = js::call_params(declaration, context_id, args);
                match self.send("Runtime.callFunctionOn", params).await {
                    Ok(response) => match js::result_value(&response) {
                        Err(e) if js::is_context_lost(&e) => continue,
                        other => return other,
                    },
                    Err(e) if js::is_context_lost(&e) => {
                        // Forget the stale context and wait for the next one.
                        self.inner
                            .state
                            .lock()
                            .unwrap()
                            .tree
                            .context_destroyed(context_id);
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
        };
        with_timeout(&*self.inner.shared.services.clock, timeout, action, attempt).await
    }

    /// Playwright's `page.evaluate(expression | function)`. Functions are
    /// called; promises are awaited; the result is deserialised into `T`.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.evaluate_value(World::Main, source, None).await?;
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
        let v = self.evaluate_value(World::Main, source, Some(&arg)).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// Playwright's `page.goto(url)`; chain options, then `.await`.
    pub fn goto(&self, url: &str) -> Goto<'_> {
        Goto {
            page: self,
            url: url.to_owned(),
            wait_until: LifecycleEvent::Load,
            timeout: None,
            referer: None,
        }
    }

    /// Waits until the main frame reached `state` (Playwright's `waitForLoadState`).
    pub async fn wait_for_load_state(
        &self,
        state: LifecycleEvent,
        timeout: Option<Duration>,
    ) -> Result<()> {
        let inner = &self.inner;
        let timeout = timeout.unwrap_or_else(|| self.default_timeout());
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            timeout,
            &format!("waiting for load state {state:?}"),
            || {
                if inner.closed.load(Ordering::SeqCst) {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                st.tree
                    .main_frame()
                    .filter(|f| f.has(state))
                    .map(|_| Ok(()))
            },
        )
        .await
    }

    async fn navigate(
        &self,
        url: &str,
        wait: LifecycleEvent,
        timeout: Duration,
        referer: Option<&str>,
    ) -> Result<Option<Response>> {
        self.ensure_open()?;
        let frame_id = self.main_frame_id()?;
        let mut params = json!({"url": url, "frameId": frame_id, "referrerPolicy": "unsafeUrl"});
        if let Some(r) = referer {
            params["referrer"] = json!(r);
        }
        let clock = &*self.inner.shared.services.clock;
        let action = format!("navigating to \"{url}\", waiting until \"{wait:?}\"");
        let r = with_timeout(clock, timeout, &action, self.send("Page.navigate", params)).await?;
        if r.get("isDownload").and_then(Value::as_bool) == Some(true) {
            return Err(Error::Navigation(format!("Download is starting at {url}")));
        }
        if let Some(text) = r
            .get("errorText")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
        {
            return Err(Error::Navigation(format!("{text} at {url}")));
        }
        let Some(loader) = r.get("loaderId").and_then(Value::as_str).map(str::to_owned) else {
            // Same-document navigation.
            return Ok(None);
        };
        let inner = &self.inner;
        wait_until(clock, &inner.changed, timeout, &action, || {
            if inner.closed.load(Ordering::SeqCst) {
                return Some(Err(Error::TargetClosed("page has been closed".into())));
            }
            let st = inner.state.lock().unwrap();
            let main = st.tree.main_frame()?;
            if main.loader_id.as_deref() != Some(loader.as_str()) || !main.has(wait) {
                return None;
            }
            Some(Ok(st.responses.get(&loader).cloned()))
        })
        .await
    }

    /// Resizes the viewport (Playwright's `page.setViewportSize`).
    pub async fn set_viewport_size(&self, viewport: Viewport) -> Result<()> {
        self.ensure_open()?;
        for (method, params) in
            viewport_commands(&self.inner.options, self.inner.window_id, Some(viewport))
        {
            self.send(method, params).await?;
        }
        self.inner.state.lock().unwrap().viewport = Some(viewport);
        Ok(())
    }

    /// The emulated viewport, if any.
    pub fn viewport_size(&self) -> Option<Viewport> {
        self.inner
            .state
            .lock()
            .unwrap()
            .viewport
            .or(self.inner.options.viewport)
    }

    /// Closes the page (and its context, if it was created by `browser.new_page()`).
    pub async fn close(&self) -> Result<()> {
        if self.is_closed() {
            return Ok(());
        }
        let context = self.inner.context.upgrade();
        if self.inner.owns_context.load(Ordering::SeqCst)
            && let Some(ctx) = &context
        {
            return crate::application::context::BrowserContext { inner: ctx.clone() }
                .close()
                .await;
        }
        let _ = self
            .inner
            .shared
            .cdp
            .call(
                None,
                "Target.closeTarget",
                json!({"targetId": self.inner.target_id}),
            )
            .await;
        self.mark_closed();
        Ok(())
    }
}

/// A pending `page.goto()`.
#[must_use = "a navigation does nothing until awaited"]
pub struct Goto<'a> {
    page: &'a Page,
    url: String,
    wait_until: LifecycleEvent,
    timeout: Option<Duration>,
    referer: Option<String>,
}

impl<'a> Goto<'a> {
    /// `commit`, `domcontentloaded`, `load` (default) or `networkidle`.
    pub fn wait_until(mut self, event: LifecycleEvent) -> Self {
        self.wait_until = event;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn referer(mut self, referer: &str) -> Self {
        self.referer = Some(referer.to_owned());
        self
    }
}

impl<'a> IntoFuture for Goto<'a> {
    type Output = Result<Option<Response>>;
    type IntoFuture = BoxFuture<'a, Self::Output>;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self
                .timeout
                .unwrap_or_else(|| self.page.navigation_timeout());
            self.page
                .traced(
                    "Frame",
                    "goto",
                    json!({"url": self.url, "timeout": timeout.as_millis() as u64}),
                    self.page.navigate(
                        &crate::domain::url_matcher::resolve_url(
                            self.page.inner.options.base_url.as_deref(),
                            &self.url,
                        ),
                        self.wait_until,
                        timeout,
                        self.referer.as_deref(),
                    ),
                )
                .await
        }
        .boxed()
    }
}

pub(crate) fn load_frame_tree(
    tree: &mut FrameTree,
    node: &Value,
    parent: Option<&str>,
    effects: &mut Vec<Effect>,
) {
    let frame = &node["frame"];
    let id = frame.get("id").and_then(Value::as_str).unwrap_or_default();
    let url = format!(
        "{}{}",
        frame.get("url").and_then(Value::as_str).unwrap_or_default(),
        frame
            .get("urlFragment")
            .and_then(Value::as_str)
            .unwrap_or_default()
    );
    let name = frame
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let loader = frame
        .get("loaderId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    effects.extend(tree.attached(id, parent));
    effects.extend(tree.navigated(id, parent, &url, name, loader));
    for child in node
        .get("childFrames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        load_frame_tree(tree, child, Some(id), effects);
    }
}

/// `scheme://host[:port]` of a URL (lowercase), for credential matching.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return String::new();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or(host);
    format!("{}://{}", scheme.to_lowercase(), host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_matching() {
        assert_eq!(
            origin_of("HTTP://User@Example.com:8080/a?b"),
            "http://example.com:8080"
        );
        assert_eq!(origin_of("https://a.test"), "https://a.test");
    }
}
