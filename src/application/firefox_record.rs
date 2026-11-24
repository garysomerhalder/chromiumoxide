//! Recording on the Firefox engine: `page.pdf()`, `recordHar`,
//! `recordVideo` and `context.tracing()`, built on BiDi
//! (`browsingContext.print`, network data collectors, screenshots).

use std::collections::HashSet;
use std::future::{Future, IntoFuture};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::application::firefox::Page;
use crate::application::har::{Attachment, write_har};
use crate::application::services::Services;
use crate::application::tracing::TracingStart;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::har_entry;
use crate::domain::firefox::init_script_function;
use crate::domain::har::{
    Creator, Entry, Har, HarContent, HarMode, HarPage, Log, RecordHar, attachment_name, iso_time,
    slim,
};
use crate::domain::pdf::PdfOptions;
use crate::domain::trace::{context_options_event, frame_snapshot, sha1_hex, zip_stored};
use crate::domain::url_matcher::UrlMatcher;
use crate::domain::video::{FPS, FrameTimeline, video_size};
use crate::ports::BidiSession;

/// A pending `page.pdf()` on Firefox: resolves to the PDF bytes.
#[must_use = "does nothing until awaited"]
pub struct Pdf<'a> {
    page: &'a Page,
    options: PdfOptions,
    path: Option<PathBuf>,
}

impl Pdf<'_> {
    pub fn options(mut self, options: PdfOptions) -> Self {
        self.options = options;
        self
    }

    /// Also writes the PDF to this file.
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl<'a> IntoFuture for Pdf<'a> {
    type Output = Result<Vec<u8>>;
    type IntoFuture = BoxFuture<'a, Result<Vec<u8>>>;

    fn into_future(self) -> Self::IntoFuture {
        async move {
            let mut params = self.options.to_bidi().map_err(Error::InvalidArgument)?;
            params["context"] = json!(self.page.main_context());
            let r = self.page.bidi_call("browsingContext.print", params).await?;
            let pdf = base64::engine::general_purpose::STANDARD
                .decode(r["data"].as_str().unwrap_or_default())
                .map_err(|e| Error::Serde(format!("invalid PDF data: {e}")))?;
            if let Some(path) = &self.path {
                self.page.services().fs.write(path, &pdf)?;
            }
            Ok(pdf)
        }
        .boxed()
    }
}

impl Page {
    /// `page.pdf()` (BiDi `browsingContext.print`; Letter, no margins).
    pub fn pdf(&self) -> Pdf<'_> {
        Pdf {
            page: self,
            options: PdfOptions::default(),
            path: None,
        }
    }
}

type PendingEntry = JoinHandle<Option<(Entry, Option<Attachment>)>>;

struct HarRec {
    options: RecordHar,
    filter: Option<UrlMatcher>,
    pages: Vec<HarPage>,
    pending: Vec<PendingEntry>,
}

struct TraceRec {
    options: TracingStart,
    start_ms: u64,
    events: Vec<String>,
    resources: Vec<(String, Vec<u8>)>,
    network: Vec<JoinHandle<Option<String>>>,
    next_call: u64,
    pages: HashSet<String>,
    /// Open `tracing.group` calls.
    groups: Vec<String>,
    /// The page global of Playwright's snapshot streamer (`snapshots: true`).
    streamer: Option<String>,
}

/// A context's recorders (HAR and tracing), shared with its pages.
pub(crate) struct Recorders {
    bidi: Arc<dyn BidiSession>,
    services: Services,
    user_context: String,
    browser_version: String,
    viewport: (u32, u32),
    /// The BiDi network data collector that keeps bodies for the recorders.
    collector: tokio::sync::Mutex<Option<String>>,
    har: Mutex<Option<HarRec>>,
    trace: Mutex<Option<TraceRec>>,
}

/// One body the data collector kept (`None` when it has none).
async fn collected(
    bidi: &Arc<dyn BidiSession>,
    collector: &str,
    request: &str,
    data_type: &str,
) -> Option<Vec<u8>> {
    let r = bidi
        .call(
            "network.getData",
            json!({"dataType": data_type, "collector": collector, "request": request}),
        )
        .await
        .ok()?;
    let bytes = &r["bytes"];
    let value = bytes["value"].as_str()?;
    if bytes["type"] == "base64" {
        base64::engine::general_purpose::STANDARD.decode(value).ok()
    } else {
        Some(value.as_bytes().to_vec())
    }
}

/// The HAR entry (and attached body) of a finished or failed request.
async fn entry(
    bidi: Arc<dyn BidiSession>,
    collector: Option<String>,
    request: Value,
    response: Option<Value>,
    pageref: String,
    policy: HarContent,
) -> (Entry, Option<Attachment>) {
    let id = request["request"].as_str().unwrap_or_default().to_owned();
    let mut post = None;
    let mut body = None;
    if let Some(collector) = &collector {
        if request["bodySize"].as_i64().unwrap_or(0) > 0 {
            post = collected(&bidi, collector, &id, "request").await;
        }
        let status = response
            .as_ref()
            .and_then(|r| r["status"].as_i64())
            .unwrap_or(0);
        if policy != HarContent::Omit && response.is_some() && !(300..400).contains(&status) {
            body = collected(&bidi, collector, &id, "response").await;
        }
    }
    let mut e = har_entry(
        &request,
        response.as_ref(),
        post.as_deref(),
        body.as_deref(),
        Some(&pageref),
    );
    let mut attachment = None;
    if policy == HarContent::Attach
        && let Some(b) = body
    {
        let content = &mut e.response.content;
        let name = attachment_name(&b, &content.mime_type);
        content.text = None;
        content.encoding = None;
        content.file = Some(name.clone());
        attachment = Some((name, b));
    }
    (e, attachment)
}

impl Recorders {
    pub(crate) fn new(
        bidi: Arc<dyn BidiSession>,
        services: Services,
        user_context: String,
        browser_version: String,
        viewport: (u32, u32),
    ) -> Self {
        Recorders {
            bidi,
            services,
            user_context,
            browser_version,
            viewport,
            collector: tokio::sync::Mutex::new(None),
            har: Mutex::new(None),
            trace: Mutex::new(None),
        }
    }

    fn recording(&self) -> bool {
        self.har.lock().unwrap().is_some() || self.trace.lock().unwrap().is_some()
    }

    /// Keeps request and response bodies of the user context from now on.
    async fn ensure_collector(&self) -> Result<()> {
        let mut collector = self.collector.lock().await;
        if collector.is_none() {
            let r = self
                .bidi
                .call(
                    "network.addDataCollector",
                    json!({"dataTypes": ["request", "response"], "maxEncodedDataSize": 50_000_000,
                           "userContexts": [self.user_context]}),
                )
                .await?;
            *collector = r["collector"].as_str().map(str::to_owned);
        }
        Ok(())
    }

    /// `recordHar`: records the context's traffic until it closes.
    pub(crate) async fn start_har(&self, options: RecordHar) -> Result<()> {
        self.ensure_collector().await?;
        *self.har.lock().unwrap() = Some(HarRec {
            filter: options.url_filter.as_deref().map(UrlMatcher::glob),
            options,
            pages: Vec::new(),
            pending: Vec::new(),
        });
        Ok(())
    }

    /// A new page of the context.
    pub(crate) fn page_created(self: &Arc<Self>, page: &Page) {
        if let Some(rec) = self.har.lock().unwrap().as_mut() {
            rec.pages.push(HarPage {
                id: page.page_id(),
                ..Default::default()
            });
        }
        self.trace_page(page);
    }

    /// One BiDi event of `page`.
    pub(crate) fn on_event(self: &Arc<Self>, page: &Page, method: &str, p: &Value) {
        if !self.recording() {
            return;
        }
        match method {
            "network.responseCompleted" | "network.fetchError" => {
                let response =
                    (method == "network.responseCompleted").then(|| p["response"].clone());
                self.network_done(page, p["request"].clone(), response);
            }
            "log.entryAdded" if p["type"] == "console" => {
                let Some(time) = self.trace_time() else {
                    return;
                };
                self.trace_event(json!({
                    "type": "console", "messageType": p["method"].as_str().unwrap_or("log"),
                    "text": p["text"], "args": [],
                    "location": {"url": "", "lineNumber": 0, "columnNumber": 0},
                    "time": time, "pageId": page.page_id(),
                }));
            }
            _ => {}
        }
    }

    fn network_done(self: &Arc<Self>, page: &Page, request: Value, response: Option<Value>) {
        let url = request["url"].as_str().unwrap_or_default().to_owned();
        let page_id = page.page_id();
        let collector = self.collector.try_lock().ok().and_then(|c| c.clone());
        if let Some(rec) = self.har.lock().unwrap().as_mut()
            && !rec.filter.as_ref().is_some_and(|f| !f.matches(&url))
        {
            let task = entry(
                self.bidi.clone(),
                collector.clone(),
                request.clone(),
                response.clone(),
                page_id.clone(),
                rec.options.content_policy(),
            );
            rec.pending.push(tokio::spawn(task.map(Some)));
        }
        if let Some(rec) = self.trace.lock().unwrap().as_mut() {
            let weak = Arc::downgrade(self);
            let task = entry(
                self.bidi.clone(),
                collector,
                request,
                response,
                page_id,
                HarContent::Embed,
            );
            rec.network.push(tokio::spawn(async move {
                let (mut entry, _) = task.await;
                // Bodies go to resources/ (`_file`), like Playwright's traces.
                let content = &mut entry.response.content;
                if content.text.is_some() {
                    let body = content.body();
                    let name = format!(
                        "resources/{}.{}",
                        sha1_hex(&body),
                        crate::domain::har::extension_for(&content.mime_type)
                    );
                    content.text = None;
                    content.encoding = None;
                    content.file = Some(name.clone());
                    if let Some(ctx) = weak.upgrade()
                        && let Some(rec) = ctx.trace.lock().unwrap().as_mut()
                        && !rec.resources.iter().any(|(n, _)| *n == name)
                    {
                        rec.resources.push((name, body));
                    }
                }
                serde_json::to_value(entry).ok().map(|snapshot| {
                    json!({"type": "resource-snapshot", "snapshot": snapshot}).to_string()
                })
            }));
        }
    }

    /// Writes the HAR (`recordHar`) before the context closes; `pages` are
    /// the context's open pages (for the page title).
    pub(crate) async fn flush_har(&self, pages: &[Page]) -> Result<()> {
        let Some(mut rec) = self.har.lock().unwrap().take() else {
            return Ok(());
        };
        let mut entries = Vec::new();
        let mut attachments: Vec<Attachment> = Vec::new();
        for handle in rec.pending.drain(..) {
            if let Ok(Some((e, a))) = handle.await {
                entries.push(e);
                if let Some(a) = a.filter(|a| !attachments.iter().any(|b| b.0 == a.0)) {
                    attachments.push(a);
                }
            }
        }
        entries.sort_by(|a, b| a.started_date_time.cmp(&b.started_date_time));
        for har_page in &mut rec.pages {
            har_page.started_date_time = entries
                .iter()
                .find(|e| e.pageref.as_deref() == Some(har_page.id.as_str()))
                .map(|e| e.started_date_time.clone())
                .unwrap_or_else(|| iso_time(0.0));
            if let Some(open) = pages.iter().find(|p| p.page_id() == har_page.id)
                && let Ok(title) = open.title().await
            {
                har_page.title = title;
            }
        }
        if rec.options.mode == HarMode::Minimal {
            rec.pages.clear();
            entries.iter_mut().for_each(slim);
        }
        let har = Har {
            log: Log {
                browser: Some(Creator {
                    name: "firefox".into(),
                    version: self.browser_version.clone(),
                }),
                pages: rec.pages,
                entries,
                ..Default::default()
            },
        };
        write_har(&*self.services.fs, &rec.options, &har, &attachments)
    }

    /// The `context-options` event that starts a trace chunk.
    fn first_event(&self, title: &str) -> String {
        context_options_event(
            "firefox",
            self.viewport.0,
            self.viewport.1,
            self.services.clock.wall_ms(),
            0.0,
            title,
            if cfg!(target_os = "macos") {
                "darwin"
            } else if cfg!(windows) {
                "win32"
            } else {
                "linux"
            },
        )
    }

    /// Records the page's frames into the trace (`snapshots: true`).
    async fn snapshot(&self, page: &Page, call_id: &str, phase: &str) {
        let Some(name) = self
            .trace
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|r| r.streamer.clone())
        else {
            return;
        };
        let main = page.main_context();
        let page_id = page.page_id();
        for frame in page.frames().await.unwrap_or_default() {
            let data = page
                .call_json_in(
                    frame.context_id(),
                    &format!(
                        "() => {{ const s = window[{name:?}]; return s ? s.captureSnapshot(undefined) : null; }}"
                    ),
                    &[],
                    None,
                )
                .await;
            let Ok(data) = data else { continue };
            if data.is_null() {
                continue;
            }
            let Some(time) = self.trace_time() else {
                return;
            };
            let (snapshot, files) = frame_snapshot(
                &data,
                call_id,
                phase,
                &page_id,
                &format!("frame@{}", frame.context_id().to_lowercase()),
                frame.context_id() == main,
                time,
            );
            if let Some(rec) = self.trace.lock().unwrap().as_mut() {
                for (name, body) in files {
                    if !rec.resources.iter().any(|(n, _)| *n == name) {
                        rec.resources.push((name, body));
                    }
                }
                rec.events
                    .push(json!({"type": "frame-snapshot", "snapshot": snapshot}).to_string());
            }
        }
    }

    fn trace_time(&self) -> Option<f64> {
        let rec = self.trace.lock().unwrap();
        let rec = rec.as_ref()?;
        Some(self.services.clock.now_ms().saturating_sub(rec.start_ms) as f64)
    }

    fn trace_event(&self, event: Value) {
        if let Some(rec) = self.trace.lock().unwrap().as_mut() {
            rec.events.push(event.to_string());
        }
    }

    /// Records an API call of `page` into the trace (`before`/`after`).
    pub(crate) async fn traced<T>(
        &self,
        page: &Page,
        class: &str,
        method: &str,
        params: Value,
        f: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let call = {
            let time = self.trace_time();
            let mut rec = self.trace.lock().unwrap();
            match (rec.as_mut(), time) {
                (Some(rec), Some(time)) => {
                    rec.next_call += 1;
                    let id = format!("call@{}", rec.next_call);
                    rec.events.push(
                        json!({"type": "before", "callId": id, "startTime": time, "class": class,
                               "method": method, "params": params, "pageId": page.page_id()})
                        .to_string(),
                    );
                    Some(id)
                }
                _ => None,
            }
        };
        if let Some(id) = &call {
            self.snapshot(page, id, "before").await;
        }
        let result = f.await;
        if let Some(id) = &call {
            self.snapshot(page, id, "after").await;
        }
        if let (Some(id), Some(time)) = (call, self.trace_time()) {
            let mut after = json!({"type": "after", "callId": id, "endTime": time});
            if let Err(e) = &result {
                after["error"] = json!({"message": e.to_string(), "name": "Error"});
            }
            self.trace_event(after);
        }
        result
    }

    /// Starts recording `page` into the trace (its screencast frames).
    fn trace_page(self: &Arc<Self>, page: &Page) {
        let screenshots = {
            let mut rec = self.trace.lock().unwrap();
            let Some(rec) = rec.as_mut() else { return };
            if !rec.pages.insert(page.page_id()) {
                return;
            }
            rec.options.screenshots
        };
        if !screenshots {
            return;
        }
        let weak = Arc::downgrade(self);
        let page = page.clone();
        tokio::spawn(async move {
            let page_id = page.page_id();
            loop {
                let frame = page.jpeg_frame().await;
                let Some(ctx) = weak.upgrade() else { break };
                let Some(time) = ctx.trace_time() else { break };
                let Ok(jpeg) = frame else { break };
                let wall = ctx.services.clock.wall_ms();
                let (w, h) = page
                    .viewport_size()
                    .map_or(ctx.viewport, |v| (v.width, v.height));
                let file = format!("screencast/{page_id}-{}.jpeg", wall as u64);
                if let Some(rec) = ctx.trace.lock().unwrap().as_mut() {
                    rec.resources.push((file.clone(), jpeg));
                    rec.events.push(
                        json!({"type": "screencast-frame", "pageId": page_id, "file": file,
                               "width": w, "height": h, "timestamp": time, "frameSwapWallTime": wall})
                        .to_string(),
                    );
                }
                drop(ctx);
                page.sleep(Duration::from_millis(100)).await;
            }
        });
    }
}

/// `context.tracing()` on Firefox.
#[derive(Clone)]
pub struct Tracing {
    recorders: Arc<Recorders>,
    pages: Vec<Page>,
}

impl Tracing {
    pub(crate) fn new(recorders: Arc<Recorders>, pages: Vec<Page>) -> Self {
        Tracing { recorders, pages }
    }

    /// `tracing.start(options)`: actions, console, network and (with
    /// `screenshots`) frames of the context's pages from now on. DOM
    /// snapshots are not recorded on Firefox yet.
    pub async fn start(&self, options: TracingStart) -> Result<()> {
        let r = &self.recorders;
        r.ensure_collector().await?;
        {
            let mut rec = r.trace.lock().unwrap();
            if rec.is_some() {
                return Err(Error::InvalidArgument(
                    "Tracing has been already started".into(),
                ));
            }
            let first = r.first_event(options.title.as_deref().unwrap_or_default());
            *rec = Some(TraceRec {
                options,
                start_ms: r.services.clock.now_ms(),
                events: vec![first],
                resources: Vec::new(),
                network: Vec::new(),
                next_call: 0,
                pages: HashSet::new(),
                groups: Vec::new(),
                streamer: None,
            });
        }
        let snapshots = r
            .trace
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|t| t.options.snapshots);
        if snapshots {
            let name = format!(
                "__playwright_snapshot_streamer_{:x}",
                (r.services.clock.wall_ms().to_bits() ^ r.services.clock.now_ms().rotate_left(13))
            );
            let script = format!(
                "({})({name:?}, true)",
                crate::application::tracing::SNAPSHOT_STREAMER
            );
            r.bidi
                .call(
                    "script.addPreloadScript",
                    json!({"functionDeclaration": init_script_function(&script),
                           "userContexts": [r.user_context]}),
                )
                .await?;
            for page in &self.pages {
                for frame in page.frames().await.unwrap_or_default() {
                    let _ = page
                        .call_json_in(
                            frame.context_id(),
                            &format!("() => {{ {script}; }}"),
                            &[],
                            None,
                        )
                        .await;
                }
            }
            if let Some(rec) = r.trace.lock().unwrap().as_mut() {
                rec.streamer = Some(name);
            }
        }
        for page in &self.pages {
            r.trace_page(page);
        }
        Ok(())
    }

    /// `tracing.startChunk({title})`: starts a new chunk of a started trace.
    pub async fn start_chunk(&self, title: Option<&str>) -> Result<()> {
        let r = &self.recorders;
        let first = r.first_event(title.unwrap_or_default());
        let now = r.services.clock.now_ms();
        let mut rec = r.trace.lock().unwrap();
        let rec = rec
            .as_mut()
            .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
        rec.events = vec![first];
        rec.resources.clear();
        rec.start_ms = now;
        Ok(())
    }

    /// `tracing.stopChunk({path})`: writes what was recorded since the last
    /// chunk; tracing goes on.
    pub async fn stop_chunk(&self, path: Option<&Path>) -> Result<()> {
        let (events, resources, network) = {
            let mut rec = self.recorders.trace.lock().unwrap();
            let rec = rec
                .as_mut()
                .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
            (
                std::mem::take(&mut rec.events),
                std::mem::take(&mut rec.resources),
                std::mem::take(&mut rec.network),
            )
        };
        self.write(path, events, resources, network).await
    }

    /// `tracing.group(name)`: groups the following actions in the viewer.
    pub async fn group(&self, name: &str) -> Result<()> {
        let time = self.recorders.trace_time();
        let mut rec = self.recorders.trace.lock().unwrap();
        let rec = rec
            .as_mut()
            .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
        rec.next_call += 1;
        let id = format!("call@{}", rec.next_call);
        rec.events.push(
            json!({"type": "before", "callId": id, "startTime": time.unwrap_or(0.0),
                   "class": "Tracing", "method": "tracingGroup", "params": {"name": name}})
            .to_string(),
        );
        rec.groups.push(id);
        Ok(())
    }

    /// `tracing.groupEnd()`.
    pub async fn group_end(&self) -> Result<()> {
        let time = self.recorders.trace_time();
        let mut rec = self.recorders.trace.lock().unwrap();
        let rec = rec
            .as_mut()
            .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
        if let Some(id) = rec.groups.pop() {
            rec.events.push(
                json!({"type": "after", "callId": id, "endTime": time.unwrap_or(0.0)}).to_string(),
            );
        }
        Ok(())
    }

    /// Writes a chunk as a trace zip when `path` is given.
    async fn write(
        &self,
        path: Option<&Path>,
        events: Vec<String>,
        resources: Vec<(String, Vec<u8>)>,
        pending: Vec<JoinHandle<Option<String>>>,
    ) -> Result<()> {
        let mut network = Vec::new();
        for handle in pending {
            if let Ok(Some(line)) = handle.await {
                network.push(line);
            }
        }
        // Bodies stored while the network entries finished.
        let mut resources = resources;
        if let Some(rec) = self.recorders.trace.lock().unwrap().as_mut() {
            for (name, body) in rec.resources.drain(..) {
                if !resources.iter().any(|(n, _)| *n == name) {
                    resources.push((name, body));
                }
            }
        }
        let Some(path) = path else {
            return Ok(());
        };
        let mut files: Vec<(&str, Vec<u8>)> = vec![
            ("trace.trace", (events.join("\n") + "\n").into_bytes()),
            ("trace.network", (network.join("\n") + "\n").into_bytes()),
            ("trace.stacks", br#"{"files":[],"stacks":[]}"#.to_vec()),
        ];
        for (name, data) in &resources {
            files.push((name.as_str(), data.clone()));
        }
        self.recorders
            .services
            .fs
            .write(path, &zip_stored(&files))?;
        Ok(())
    }

    /// `tracing.stop({path})`: writes the trace zip when `path` is given.
    pub async fn stop(&self, path: Option<&Path>) -> Result<()> {
        let Some(rec) = self.recorders.trace.lock().unwrap().take() else {
            return Err(Error::InvalidArgument("Tracing is not started".into()));
        };
        self.write(path, rec.events, rec.resources, rec.network)
            .await
    }
}

/// The video of a Firefox page (`page.video()`). Cheap to clone.
#[derive(Clone)]
pub struct Video {
    path: PathBuf,
    services: Services,
    /// `None` while recording; then the outcome.
    done: watch::Sender<Option<std::result::Result<(), String>>>,
}

impl std::fmt::Debug for Video {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Video")
            .field("path", &self.path)
            .finish()
    }
}

impl Video {
    async fn finished(&self) -> Result<()> {
        let mut rx = self.done.subscribe();
        loop {
            if let Some(outcome) = rx.borrow().clone() {
                return outcome.map_err(Error::Io);
            }
            if rx.changed().await.is_err() {
                return Err(Error::TargetClosed("video recording was dropped".into()));
            }
        }
    }

    /// Waits until the page is closed and the file is written; its path.
    pub async fn path(&self) -> Result<PathBuf> {
        self.finished().await?;
        Ok(self.path.clone())
    }

    /// Copies the finished video to `path`.
    pub async fn save_as(&self, path: impl AsRef<Path>) -> Result<()> {
        self.finished().await?;
        let data = self.services.fs.read(&self.path)?;
        self.services.fs.write(path.as_ref(), &data)?;
        Ok(())
    }

    /// Deletes the video file (after recording ends).
    pub async fn delete(&self) -> Result<()> {
        let _ = self.finished().await;
        self.services.fs.remove_file(&self.path)?;
        Ok(())
    }

    /// Starts recording `page` (`recordVideo`): screenshots, as fast as
    /// Firefox takes them, on a constant-rate timeline.
    pub(crate) fn start(page: &Page) -> Option<Video> {
        let options = page.options().record_video.clone()?;
        let viewport = page.viewport_size();
        let (w, h) = video_size(viewport, options.size);
        let fps = options.fps.unwrap_or(FPS);
        let dpr = page.options().device_scale_factor.unwrap_or(1.0);
        let css = viewport.map_or((1280, 720), |v| (v.width, v.height));
        let source = (
            (f64::from(css.0) * dpr).round() as u32,
            (f64::from(css.1) * dpr).round() as u32,
        );
        let path = options
            .dir
            .join(format!("{}.webm", page.main_context().to_lowercase()));
        let (done, _) = watch::channel(None);
        let video = Video {
            path: path.clone(),
            services: page.services().clone(),
            done,
        };
        let page = page.clone();
        let recording = video.clone();
        tokio::spawn(async move {
            let services = page.services().clone();
            let clock = services.clock.clone();
            let outcome = async {
                let mut sink = services
                    .video
                    .start_scaled(&path, w, h, fps, source)
                    .await
                    .map_err(|e| format!("cannot start video encoder: {e}"))?;
                let mut timeline = FrameTimeline::new(clock.now_ms(), fps);
                let mut last: Option<Vec<u8>> = None;
                while let Ok(frame) = page.jpeg_frame().await {
                    let now = clock.now_ms();
                    if let Some(prev) = &last {
                        for _ in 0..timeline.frames_before(now) {
                            sink.write_frame(prev).await.map_err(|e| e.to_string())?;
                        }
                    }
                    timeline.push(now);
                    last = Some(frame);
                }
                let frame = last.unwrap_or_else(|| {
                    base64::engine::general_purpose::STANDARD
                        .decode(crate::application::video::WHITE_JPEG)
                        .unwrap_or_default()
                });
                if !timeline.has_frame() {
                    timeline.push(clock.now_ms());
                }
                for _ in 0..timeline.padding_at_close(clock.now_ms()) {
                    sink.write_frame(&frame).await.map_err(|e| e.to_string())?;
                }
                sink.finish().await.map_err(|e| e.to_string())
            }
            .await;
            recording.done.send_replace(Some(outcome));
        });
        Some(video)
    }
}

impl Page {
    /// The viewport as a JPEG (a video or trace frame); fails once the page
    /// is closed.
    pub(crate) async fn jpeg_frame(&self) -> Result<Vec<u8>> {
        let r = self
            .bidi_call(
                "browsingContext.captureScreenshot",
                json!({"context": self.main_context(), "origin": "viewport",
                       "format": {"type": "image/jpeg", "quality": 0.9}}),
            )
            .await?;
        base64::engine::general_purpose::STANDARD
            .decode(r["data"].as_str().unwrap_or_default())
            .map_err(|e| Error::Serde(format!("invalid screenshot data: {e}")))
    }
}
