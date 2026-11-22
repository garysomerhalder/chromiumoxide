//! Tracing (Playwright's `context.tracing`): actions, console, network and
//! screencast frames, written as a Playwright-style `trace.zip`.

use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures::StreamExt;
use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::events::PageEvent;
use crate::application::page::Page;
use crate::domain::emulation::FontPlatform;
use crate::domain::error::{Error, Result};
use crate::domain::trace::{context_options_event, zip_stored};

/// Options for `tracing.start()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TracingStart {
    /// Record screencast frames (filmstrip).
    pub screenshots: bool,
    /// DOM snapshots: not recorded yet (accepted for compatibility).
    pub snapshots: bool,
    pub name: Option<String>,
    pub title: Option<String>,
}

pub(crate) struct TraceRecording {
    options: TracingStart,
    start_ms: u64,
    events: Vec<String>,
    resources: Vec<(String, Vec<u8>)>,
    network: Vec<tokio::task::JoinHandle<Option<String>>>,
    next_call: u64,
    pages: HashSet<String>,
    groups: Vec<String>,
}

pub(crate) type TracingState = Mutex<Option<TraceRecording>>;

/// `context.tracing`.
#[derive(Clone)]
pub struct Tracing {
    context: Arc<ContextInner>,
}

impl std::fmt::Debug for Tracing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracing").finish()
    }
}

impl BrowserContext {
    pub fn tracing(&self) -> Tracing {
        Tracing {
            context: self.inner.clone(),
        }
    }
}

fn platform(p: FontPlatform) -> &'static str {
    match p {
        FontPlatform::Linux => "linux",
        FontPlatform::Mac => "darwin",
        FontPlatform::Win => "win32",
    }
}

impl ContextInner {
    fn trace_time(&self) -> Option<f64> {
        let rec = self.tracing.lock().unwrap();
        let rec = rec.as_ref()?;
        Some(
            self.shared
                .services
                .clock
                .now_ms()
                .saturating_sub(rec.start_ms) as f64,
        )
    }

    fn trace_event(&self, event: Value) {
        if let Some(rec) = self.tracing.lock().unwrap().as_mut() {
            rec.events.push(event.to_string());
        }
    }

    /// Starts recording a page into the trace (console, network, frames).
    pub(crate) fn trace_page(self: &Arc<Self>, page: &Page) {
        let screenshots = {
            let mut rec = self.tracing.lock().unwrap();
            let Some(rec) = rec.as_mut() else { return };
            if !rec.pages.insert(page.target_id().to_owned()) {
                return;
            }
            rec.options.screenshots
        };
        let page_id = format!("page@{}", page.target_id().to_lowercase());
        let weak = Arc::downgrade(self);
        let mut events = page.events();
        let id = page_id.clone();
        tokio::spawn(async move {
            while let Some(e) = events.next().await {
                let Some(ctx) = weak.upgrade() else { break };
                let Some(time) = ctx.trace_time() else { break };
                match e {
                    PageEvent::Console(m) => ctx.trace_event(json!({
                        "type": "console", "messageType": m.kind(), "text": m.text(), "args": [],
                        "location": {"url": m.location().url, "lineNumber": m.location().line_number,
                                     "columnNumber": m.location().column_number},
                        "time": time, "pageId": id,
                    })),
                    PageEvent::RequestFinished(r) | PageEvent::RequestFailed(r) => {
                        let mut rec = ctx.tracing.lock().unwrap();
                        if let Some(rec) = rec.as_mut() {
                            let id = id.clone();
                            rec.network.push(tokio::spawn(async move {
                                let entry =
                                    crate::application::har::entry_for(r, id, false).await;
                                serde_json::to_value(entry).ok().map(|snapshot| {
                                    json!({"type": "resource-snapshot", "snapshot": snapshot})
                                        .to_string()
                                })
                            }));
                        }
                    }
                    PageEvent::Close => break,
                    _ => {}
                }
            }
        });
        if screenshots && self.options.record_video.is_none() {
            self.trace_screencast(page, page_id);
        }
    }

    fn trace_screencast(self: &Arc<Self>, page: &Page, page_id: String) {
        let (w, h) = page
            .inner
            .options
            .viewport
            .map_or((800, 600), |v| (v.width, v.height));
        let page = page.clone();
        let weak = Arc::downgrade(self);
        let mut cdp_events = page.inner.shared.cdp.subscribe();
        tokio::spawn(async move {
            let _ = page
                .send(
                    "Page.startScreencast",
                    json!({"format": "jpeg", "quality": 90, "maxWidth": w, "maxHeight": h}),
                )
                .await;
            while let Some(e) = cdp_events.next().await {
                if page.is_closed() {
                    break;
                }
                if e.session_id.as_deref() != Some(page.inner.session_id.as_str())
                    || e.method != "Page.screencastFrame"
                {
                    continue;
                }
                if let Some(sid) = e.params.get("sessionId").cloned() {
                    tokio::spawn(page.inner.shared.cdp.call(
                        Some(&page.inner.session_id),
                        "Page.screencastFrameAck",
                        json!({"sessionId": sid}),
                    ));
                }
                let Some(ctx) = weak.upgrade() else { break };
                let Some(time) = ctx.trace_time() else { break };
                let data = e
                    .params
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Ok(jpeg) = base64::engine::general_purpose::STANDARD.decode(data) else {
                    continue;
                };
                let meta = &e.params["metadata"];
                let wall = meta.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0) * 1000.0;
                let file = format!("screencast/{page_id}-{}.jpeg", wall as u64);
                let mut rec = ctx.tracing.lock().unwrap();
                let Some(rec) = rec.as_mut() else { break };
                rec.resources.push((file.clone(), jpeg));
                rec.events.push(
                    json!({
                        "type": "screencast-frame", "pageId": page_id, "file": file,
                        "width": meta.get("deviceWidth").and_then(Value::as_f64).unwrap_or(w as f64),
                        "height": meta.get("deviceHeight").and_then(Value::as_f64).unwrap_or(h as f64),
                        "timestamp": time, "frameSwapWallTime": wall,
                    })
                    .to_string(),
                );
            }
            let _ = page.send("Page.stopScreencast", json!({})).await;
        });
    }
}

impl Tracing {
    /// `tracing.start(options)`.
    pub async fn start(&self, options: TracingStart) -> Result<()> {
        let clock = self.context.shared.services.clock.clone();
        {
            let mut rec = self.context.tracing.lock().unwrap();
            if rec.is_some() {
                return Err(Error::InvalidArgument(
                    "Tracing has been already started".into(),
                ));
            }
            let (w, h) = self
                .context
                .options
                .viewport
                .map_or((1280, 720), |v| (v.width, v.height));
            let first = context_options_event(
                "chromium",
                w,
                h,
                clock.wall_ms(),
                0.0,
                options.title.as_deref().unwrap_or_default(),
                platform(self.context.shared.font_platform),
            );
            *rec = Some(TraceRecording {
                options,
                start_ms: clock.now_ms(),
                events: vec![first],
                resources: Vec::new(),
                network: Vec::new(),
                next_call: 1,
                pages: HashSet::new(),
                groups: Vec::new(),
            });
        }
        let pages: Vec<Page> = self.context.pages.lock().unwrap().clone();
        for page in pages {
            self.context.trace_page(&page);
        }
        Ok(())
    }

    fn first_event(&self, title: &str) -> String {
        let (w, h) = self
            .context
            .options
            .viewport
            .map_or((1280, 720), |v| (v.width, v.height));
        context_options_event(
            "chromium",
            w,
            h,
            self.context.shared.services.clock.wall_ms(),
            0.0,
            title,
            platform(self.context.shared.font_platform),
        )
    }

    async fn write_chunk(
        &self,
        events: Vec<String>,
        resources: Vec<(String, Vec<u8>)>,
        pending: Vec<tokio::task::JoinHandle<Option<String>>>,
        path: &Path,
    ) -> Result<()> {
        let mut network = Vec::new();
        for handle in pending {
            if let Ok(Some(line)) = handle.await {
                network.push(line);
            }
        }
        let mut files: Vec<(&str, Vec<u8>)> = vec![
            ("trace.trace", (events.join("\n") + "\n").into_bytes()),
            ("trace.network", (network.join("\n") + "\n").into_bytes()),
            ("trace.stacks", br#"{"files":[],"stacks":[]}"#.to_vec()),
        ];
        for (name, data) in &resources {
            files.push((name.as_str(), data.clone()));
        }
        let zip = zip_stored(&files);
        self.context.shared.services.fs.write(path, &zip)?;
        Ok(())
    }

    /// `tracing.stop({path})`: writes the trace when `path` is given.
    pub async fn stop(&self, path: Option<&Path>) -> Result<()> {
        let Some(rec) = self.context.tracing.lock().unwrap().take() else {
            return Err(Error::InvalidArgument("Tracing is not started".into()));
        };
        match path {
            Some(path) => {
                self.write_chunk(rec.events, rec.resources, rec.network, path)
                    .await
            }
            None => Ok(()),
        }
    }

    /// `tracing.startChunk({title})`: starts a new chunk of a started trace.
    pub async fn start_chunk(&self, title: Option<&str>) -> Result<()> {
        let first = self.first_event(title.unwrap_or_default());
        let now = self.context.shared.services.clock.now_ms();
        let mut rec = self.context.tracing.lock().unwrap();
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
        let (events, resources, pending) = {
            let mut rec = self.context.tracing.lock().unwrap();
            let rec = rec
                .as_mut()
                .ok_or_else(|| Error::InvalidArgument("Tracing is not started".into()))?;
            (
                std::mem::take(&mut rec.events),
                std::mem::take(&mut rec.resources),
                std::mem::take(&mut rec.network),
            )
        };
        match path {
            Some(path) => self.write_chunk(events, resources, pending, path).await,
            None => Ok(()),
        }
    }

    /// `tracing.group(name)`: groups the following actions in the viewer.
    pub async fn group(&self, name: &str) -> Result<()> {
        let time = self.context.trace_time();
        let mut rec = self.context.tracing.lock().unwrap();
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
        let time = self.context.trace_time();
        let mut rec = self.context.tracing.lock().unwrap();
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

    /// `tracing.startHar(path)`: records a HAR from now on.
    pub async fn start_har(&self, path: impl AsRef<Path>) -> Result<()> {
        {
            let mut har = self.context.har.lock().unwrap();
            if har.is_some() {
                return Err(Error::InvalidArgument(
                    "HAR recording is already started".into(),
                ));
            }
            *har = crate::application::har::recording(&Some(crate::domain::har::RecordHar::new(
                path.as_ref(),
            )))
            .into_inner()
            .unwrap();
        }
        let pages: Vec<Page> = self.context.pages.lock().unwrap().clone();
        for page in pages {
            self.context.har_track_page(&page);
        }
        Ok(())
    }

    /// `tracing.stopHar()`: writes the HAR started with `start_har`.
    pub async fn stop_har(&self) -> Result<()> {
        BrowserContext {
            inner: self.context.clone(),
        }
        .flush_har()
        .await
    }
}

impl Page {
    /// Runs an API call, recording it as a trace action when tracing.
    pub(crate) async fn traced<T, F>(
        &self,
        class: &str,
        method: &str,
        params: Value,
        f: F,
    ) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        self.inner
            .calls
            .lock()
            .unwrap()
            .push((method.to_owned(), params.clone()));
        let r = self.traced_call(class, method, params, f).await;
        self.inner.calls.lock().unwrap().pop();
        r
    }

    async fn traced_call<T, F>(&self, class: &str, method: &str, params: Value, f: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        let Some(ctx) = self.inner.context.upgrade() else {
            return f.await;
        };
        let call_id = {
            let mut rec = ctx.tracing.lock().unwrap();
            match rec.as_mut() {
                Some(rec) => {
                    rec.next_call += 1;
                    Some(format!("call@{}", rec.next_call))
                }
                None => None,
            }
        };
        let Some(call_id) = call_id else {
            return f.await;
        };
        if let Some(t) = ctx.trace_time() {
            ctx.trace_event(json!({
                "type": "before", "callId": call_id, "startTime": t, "class": class,
                "method": method, "params": params, "pageId": format!("page@{}", self.target_id().to_lowercase()),
            }));
        }
        let result = f.await;
        if let Some(t) = ctx.trace_time() {
            let mut after = json!({"type": "after", "callId": call_id, "endTime": t});
            if let Err(e) = &result {
                after["error"] = json!({"message": e.to_string(), "name": "Error"});
            }
            ctx.trace_event(after);
        }
        result
    }
}
