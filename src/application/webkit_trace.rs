//! HAR recording (`recordHar`, `tracing.startHar`) and tracing
//! (`context.tracing`: actions, console, network, screencast frames, DOM
//! snapshots, chunks and groups) on the WebKit engine, as on Firefox: the
//! HAR entries and trace resources come from WebKit's live `Request`
//! objects.

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::application::har::{Attachment, write_har};
use crate::application::services::Services;
use crate::application::tracing::TracingStart;
use crate::application::webkit::{BrowserContext, Page};
use crate::application::webkit_events::PageEvent;
use crate::application::webkit_network::Request;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::har_entry;
use crate::domain::har::{
    Creator, Entry, Har, HarContent, HarMode, HarPage, Log, RecordHar, attachment_name, iso_time,
    slim,
};
use crate::domain::screenshot::{ImageFormat, ScreenshotOptions};
use crate::domain::trace::{context_options_event, frame_snapshot, sha1_hex, zip_stored};
use crate::domain::url_matcher::UrlMatcher;

/// The HAR entry (and attached body) of a finished or failed request.
async fn entry(
    request: Request,
    pageref: String,
    policy: HarContent,
) -> (Entry, Option<Attachment>) {
    // A request that never ends is recorded as it is.
    let failure = tokio::time::timeout(Duration::from_secs(10), request.settled())
        .await
        .unwrap_or(Some("not finished".into()));
    let post = request.post_data_buffer().await.ok().flatten();
    let response = request.existing_response();
    let mut body = None;
    if let Some(r) = &response
        && policy != HarContent::Omit
        && failure.is_none()
        && !(300..400).contains(&r.status())
    {
        body = r.body().await.ok();
    }
    let (req, res) = request.har_json(response.as_ref());
    let mut e = har_entry(
        &req,
        res.as_ref(),
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
    services: Services,
    browser_version: String,
    viewport: (u32, u32),
    har: Mutex<Option<HarRec>>,
    trace: Mutex<Option<TraceRec>>,
}

impl Recorders {
    pub(crate) fn new(services: Services, browser_version: String, viewport: (u32, u32)) -> Self {
        Recorders {
            services,
            browser_version,
            viewport,
            har: Mutex::new(None),
            trace: Mutex::new(None),
        }
    }

    pub(crate) fn tracing(&self) -> bool {
        self.trace.lock().unwrap().is_some()
    }

    fn recording(&self) -> bool {
        self.har.lock().unwrap().is_some() || self.trace.lock().unwrap().is_some()
    }

    /// `recordHar`: records the context's traffic until it closes.
    pub(crate) async fn start_har(&self, options: RecordHar) -> Result<()> {
        let mut har = self.har.lock().unwrap();
        if har.is_some() {
            return Err(crate::domain::error::Error::InvalidArgument(
                "HAR recording is already started".into(),
            ));
        }
        *har = Some(HarRec {
            filter: options.url_filter.as_deref().map(UrlMatcher::glob),
            options,
            pages: Vec::new(),
            pending: Vec::new(),
        });
        Ok(())
    }

    /// A new page of the context.
    pub(crate) fn page_created(self: &Arc<Self>, page: &Page) {
        self.har_page(page);
        self.trace_page(page);
    }

    /// Adds `page` to the HAR being recorded.
    pub(crate) fn har_page(&self, page: &Page) {
        if let Some(rec) = self.har.lock().unwrap().as_mut()
            && !rec.pages.iter().any(|p| p.id == page.page_id())
        {
            rec.pages.push(HarPage {
                id: page.page_id(),
                ..Default::default()
            });
        }
    }

    /// A page event, for the recorders.
    pub(crate) fn on_event(self: &Arc<Self>, page: &Page, event: &PageEvent) {
        if !self.recording() {
            return;
        }
        match event {
            // Recorded from the start; the entry waits for the end.
            PageEvent::Request(r) => self.network_done(page, r.clone()),
            PageEvent::Console(m) => {
                let Some(time) = self.trace_time() else {
                    return;
                };
                self.trace_event(json!({
                    "type": "console", "messageType": m.kind(), "text": m.text(), "args": [],
                    "location": {"url": m.location().url, "lineNumber": m.location().line_number,
                                 "columnNumber": m.location().column_number},
                    "time": time, "pageId": page.page_id(),
                }));
            }
            _ => {}
        }
    }

    fn network_done(self: &Arc<Self>, page: &Page, request: Request) {
        let url = request.url().to_owned();
        let page_id = page.page_id();
        if let Some(rec) = self.har.lock().unwrap().as_mut()
            && !rec.filter.as_ref().is_some_and(|f| !f.matches(&url))
        {
            let task = entry(
                request.clone(),
                page_id.clone(),
                rec.options.content_policy(),
            );
            rec.pending.push(tokio::spawn(task.map(Some)));
        }
        if let Some(rec) = self.trace.lock().unwrap().as_mut() {
            let weak = Arc::downgrade(self);
            let task = entry(request, page_id, HarContent::Embed);
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
                    name: "webkit".into(),
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
            "webkit",
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
        let page_id = page.page_id();
        for frame in page.frames().await.unwrap_or_default() {
            let data = page
                .evaluate_value_in(
                    &format!(
                        "() => {{ const s = window[{name:?}]; return s ? s.captureSnapshot(undefined) : null; }}"
                    ),
                    None,
                    frame.scope(),
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
                &format!("frame@{}", frame.id.to_lowercase()),
                frame.is_main(),
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

/// `context.tracing()` on WebKit.
#[derive(Clone)]
pub struct Tracing {
    recorders: Arc<Recorders>,
    pages: Vec<Page>,
    context: BrowserContext,
}

impl Tracing {
    pub(crate) fn new(context: &BrowserContext) -> Self {
        Tracing {
            recorders: context.inner.recorders.clone(),
            pages: context.live_pages(),
            context: context.clone(),
        }
    }

    /// `tracing.startHar(path)`: records the context's traffic as a HAR
    /// until `stop_har`.
    pub async fn start_har(&self, path: impl AsRef<Path>) -> Result<()> {
        self.start_har_with(path, crate::domain::har::HarOptions::default())
            .await
    }

    /// `tracing.startHar(path, { content, mode, resourcesDir, urlFilter })`.
    pub async fn start_har_with(
        &self,
        path: impl AsRef<Path>,
        options: crate::domain::har::HarOptions,
    ) -> Result<()> {
        self.recorders
            .start_har(RecordHar::with_options(path.as_ref(), options))
            .await?;
        for page in &self.pages {
            self.recorders.har_page(page);
        }
        Ok(())
    }

    /// `tracing.stopHar()`: writes the HAR started with `start_har`.
    pub async fn stop_har(&self) -> Result<()> {
        self.recorders.flush_har(&self.pages).await
    }

    /// `tracing.start(options)`: actions, console, network, (with
    /// `screenshots`) frames and (with `snapshots`) DOM snapshots of the
    /// context's pages from now on.
    pub async fn start(&self, options: TracingStart) -> Result<()> {
        let r = &self.recorders;
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
            self.context.add_init_script(&script).await?;
            for page in &self.pages {
                for frame in page.frames().await.unwrap_or_default() {
                    let _ = page
                        .evaluate_value_in(&format!("void ({script})"), None, frame.scope())
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

impl BrowserContext {
    /// `context.tracing()`.
    pub fn tracing(&self) -> Tracing {
        Tracing::new(self)
    }
}

impl Page {
    /// The page's id in traces and HARs.
    pub(crate) fn page_id(&self) -> String {
        format!("page@{}", self.inner.proxy.to_lowercase())
    }

    /// A JPEG of the viewport (trace screencast frames).
    pub(crate) async fn jpeg_frame(&self) -> Result<Vec<u8>> {
        self.screenshot()
            .options(ScreenshotOptions {
                format: ImageFormat::Jpeg,
                quality: Some(90),
                ..Default::default()
            })
            .await
    }

    /// Runs an API call, recorded in the context's trace when tracing.
    pub(crate) async fn traced<T>(
        &self,
        class: &str,
        method: &str,
        params: Value,
        f: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        match self.inner.context.upgrade() {
            Some(ctx) if ctx.recorders.tracing() => {
                ctx.recorders
                    .clone()
                    .traced(self, class, method, params, f)
                    .await
            }
            _ => f.await,
        }
    }
}
