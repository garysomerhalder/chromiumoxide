//! `page.screencast`: JPEG frames from the page, optionally recorded to a
//! `.webm` file, and user overlays (Playwright's `Screencast`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use base64::Engine;
use futures::StreamExt;
use futures::channel::oneshot;
use serde_json::{Value, json};

use crate::application::events::PageEvent;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::video::FrameTimeline;

/// One screencast frame.
#[derive(Debug, Clone)]
pub struct ScreencastFrame {
    /// JPEG bytes.
    pub data: Vec<u8>,
    /// When the frame was produced, in milliseconds since the Unix epoch.
    pub timestamp: f64,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

type FrameCallback = Arc<dyn Fn(ScreencastFrame) + Send + Sync>;

/// Options of `screencast.start()`.
#[derive(Clone, Default)]
pub struct ScreencastOptions {
    path: Option<PathBuf>,
    size: Option<(u32, u32)>,
    quality: Option<u8>,
    on_frame: Option<FrameCallback>,
}

impl std::fmt::Debug for ScreencastOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScreencastOptions")
            .field("path", &self.path)
            .field("size", &self.size)
            .field("quality", &self.quality)
            .finish()
    }
}

impl ScreencastOptions {
    /// Records a `.webm` video to this file (needs ffmpeg, like `recordVideo`).
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.path = Some(path.into());
        self
    }
    /// Frame size (default: the viewport scaled to fit 800x800).
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.size = Some((width, height));
        self
    }
    /// JPEG quality, 0-100 (default 90).
    pub fn quality(mut self, quality: u8) -> Self {
        self.quality = Some(quality);
        self
    }
    /// Called with every frame.
    pub fn on_frame(mut self, f: impl Fn(ScreencastFrame) + Send + Sync + 'static) -> Self {
        self.on_frame = Some(Arc::new(f));
        self
    }
}

/// Per-page screencast state.
#[derive(Default)]
pub(crate) struct ScreencastState {
    running: Option<Running>,
    /// User overlays, in insertion order.
    overlays: Vec<(String, String)>,
    /// The task re-adding overlays after navigations is running.
    watching: bool,
}

struct Running {
    stop: oneshot::Sender<()>,
    done: oneshot::Receiver<std::result::Result<(), String>>,
}

static OVERLAY_IDS: AtomicU64 = AtomicU64::new(1);

/// `page.screencast`. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Screencast {
    page: Page,
}

/// An overlay from `show_overlay`; `remove()` takes it away.
#[derive(Clone, Debug)]
pub struct Overlay {
    page: Page,
    id: String,
}

impl Overlay {
    pub async fn remove(&self) -> Result<()> {
        Screencast {
            page: self.page.clone(),
        }
        .remove_overlay(&self.id)
        .await
    }
}

impl Page {
    /// `page.screencast`.
    pub fn screencast(&self) -> Screencast {
        Screencast { page: self.clone() }
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const CHAPTER_FADE_MS: u64 = 300;

fn chapter_html(title: &str, description: Option<&str>) -> String {
    let fade = CHAPTER_FADE_MS;
    let description = description
        .map(|d| format!("<div id=\"description\">{}</div>", escape_html(d)))
        .unwrap_or_default();
    format!(
        "<style>
      @keyframes pw-chapter-fade-in {{ from {{ opacity: 0; }} to {{ opacity: 1; }} }}
      @keyframes pw-chapter-fade-out {{ from {{ opacity: 1; }} to {{ opacity: 0; }} }}
      #background {{ position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;
        backdrop-filter: blur(2px); animation: pw-chapter-fade-in {fade}ms ease-out forwards; }}
      #background.fade-out {{ animation: pw-chapter-fade-out {fade}ms ease-in forwards; }}
      #content {{ background: rgba(0, 0, 0, 0.7); border: 1px solid rgba(255, 255, 255, 0.15); border-radius: 16px;
        padding: 40px 56px; max-width: 560px; box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4); }}
      #title {{ color: white; font-family: system-ui, -apple-system, sans-serif; font-size: 28px; font-weight: 600;
        line-height: 1.3; text-align: center; letter-spacing: -0.01em; }}
      #description {{ color: rgba(255, 255, 255, 0.7); font-family: system-ui, -apple-system, sans-serif;
        font-size: 15px; line-height: 1.5; margin-top: 12px; text-align: center; }}
    </style><div id=\"background\"><div id=\"content\"><div id=\"title\">{}</div>{description}</div></div>",
        escape_html(title)
    )
}

impl Screencast {
    fn sleep(&self, d: Duration) -> futures::future::BoxFuture<'static, ()> {
        self.page.inner.shared.services.clock.sleep(d)
    }

    /// Starts the screencast (`screencast.start()`).
    pub async fn start(&self, options: ScreencastOptions) -> Result<()> {
        let page = self.page.clone();
        let (stop, stop_rx) = oneshot::channel();
        let (done_tx, done) = oneshot::channel();
        {
            let mut st = page.inner.screencast.lock().unwrap();
            if st.running.is_some() {
                return Err(Error::InvalidArgument(
                    "Screencast is already started".into(),
                ));
            }
            st.running = Some(Running { stop, done });
        }
        let (w, h) = options.size.unwrap_or_else(|| {
            let v = page
                .inner
                .options
                .viewport
                .unwrap_or(crate::domain::emulation::Viewport {
                    width: 800,
                    height: 600,
                });
            let scale = (800.0 / v.width.max(v.height) as f64).min(1.0);
            (
                (v.width as f64 * scale).floor() as u32,
                (v.height as f64 * scale).floor() as u32,
            )
        });
        let (w, h) = (w & !1, h & !1);
        // A video recording already runs Chromium's screencast for this page.
        let shared_with_video = page.video().is_some();
        let mut cdp_events = page.inner.shared.cdp.subscribe();
        if !shared_with_video {
            page.send(
                "Page.startScreencast",
                json!({"format": "jpeg", "quality": options.quality.unwrap_or(90),
                       "maxWidth": w, "maxHeight": h, "everyNthFrame": 1}),
            )
            .await?;
        }
        let mut sink = match &options.path {
            Some(path) => Some(
                page.inner
                    .shared
                    .services
                    .video
                    .start(path, w, h)
                    .await
                    .map_err(|e| Error::Io(format!("cannot start video encoder: {e}")))?,
            ),
            None => None,
        };
        let mut page_events = page.events();
        let on_frame = options.on_frame.clone();
        tokio::spawn(async move {
            let clock = page.inner.shared.services.clock.clone();
            let mut stop_rx = stop_rx;
            let outcome = async {
                let mut timeline = FrameTimeline::new(clock.now_ms());
                let mut last: Option<Vec<u8>> = None;
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => break,
                        e = page_events.next() => {
                            if matches!(e, None | Some(PageEvent::Close)) {
                                break;
                            }
                        }
                        e = cdp_events.next() => {
                            let Some(e) = e else { break };
                            if e.session_id.as_deref() != Some(page.inner.session_id.as_str())
                                || e.method != "Page.screencastFrame"
                            {
                                continue;
                            }
                            if !shared_with_video
                                && let Some(sid) = e.params.get("sessionId").cloned()
                            {
                                tokio::spawn(page.inner.shared.cdp.call(
                                    Some(&page.inner.session_id),
                                    "Page.screencastFrameAck",
                                    json!({"sessionId": sid}),
                                ));
                            }
                            let data = e.params.get("data").and_then(Value::as_str).unwrap_or_default();
                            let Ok(frame) = base64::engine::general_purpose::STANDARD.decode(data) else {
                                continue;
                            };
                            let meta = &e.params["metadata"];
                            let num = |k: &str| meta.get(k).and_then(Value::as_f64).unwrap_or_default();
                            if let Some(cb) = &on_frame {
                                let ts = num("timestamp");
                                cb(ScreencastFrame {
                                    data: frame.clone(),
                                    timestamp: if ts > 0.0 { ts * 1000.0 } else { clock.wall_ms() },
                                    viewport_width: num("deviceWidth").round() as u32,
                                    viewport_height: num("deviceHeight").round() as u32,
                                });
                            }
                            if let Some(sink) = sink.as_mut() {
                                let now = clock.now_ms();
                                if let Some(prev) = &last {
                                    for _ in 0..timeline.frames_before(now) {
                                        sink.write_frame(prev).await.map_err(|e| e.to_string())?;
                                    }
                                }
                                timeline.push(now);
                                last = Some(frame);
                            }
                        }
                    }
                }
                if !shared_with_video && !page.is_closed() {
                    let _ = page.send("Page.stopScreencast", json!({})).await;
                }
                if let Some(mut sink) = sink {
                    if let Some(frame) = last {
                        for _ in 0..timeline.padding_at_close(clock.now_ms()) {
                            sink.write_frame(&frame).await.map_err(|e| e.to_string())?;
                        }
                    }
                    sink.finish().await.map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            .await;
            let _ = done_tx.send(outcome);
        });
        Ok(())
    }

    /// Stops the screencast; a recording is finished and saved.
    pub async fn stop(&self) -> Result<()> {
        let running = self.page.inner.screencast.lock().unwrap().running.take();
        let Some(running) = running else {
            return Ok(());
        };
        let _ = running.stop.send(());
        match running.done.await {
            Ok(Ok(())) | Err(_) => Ok(()),
            Ok(Err(e)) => Err(Error::Io(e)),
        }
    }

    async fn add_overlay_in_page(page: &Page, id: &str, html: &str) -> Result<()> {
        page.injected_call(
            None,
            "(i, id, html) => { i._ensureHighlight().install(); if (!i.getUserOverlay(id)) i.addUserOverlay(id, html); }",
            &[json!(id), json!(html)],
            true,
            page.default_timeout(),
        )
        .await?;
        Ok(())
    }

    fn watch_navigations(&self) {
        {
            let mut st = self.page.inner.screencast.lock().unwrap();
            if st.watching {
                return;
            }
            st.watching = true;
        }
        let page = self.page.clone();
        let mut events = page.events();
        tokio::spawn(async move {
            while let Some(e) = events.next().await {
                match e {
                    PageEvent::Close => break,
                    PageEvent::DomContentLoaded | PageEvent::Load => {
                        let overlays = page.inner.screencast.lock().unwrap().overlays.clone();
                        for (id, html) in overlays {
                            let _ = Self::add_overlay_in_page(&page, &id, &html).await;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    async fn show(&self, html: &str) -> Result<String> {
        let id = format!("overlay-{}", OVERLAY_IDS.fetch_add(1, Ordering::SeqCst));
        self.page
            .inner
            .screencast
            .lock()
            .unwrap()
            .overlays
            .push((id.clone(), html.to_owned()));
        self.watch_navigations();
        Self::add_overlay_in_page(&self.page, &id, html).await?;
        Ok(id)
    }

    async fn remove_overlay(&self, id: &str) -> Result<()> {
        self.page
            .inner
            .screencast
            .lock()
            .unwrap()
            .overlays
            .retain(|(i, _)| i != id);
        let _ = self
            .page
            .injected_call(
                None,
                "(i, id) => { i.removeUserOverlay(id); }",
                &[json!(id)],
                true,
                self.page.default_timeout(),
            )
            .await;
        Ok(())
    }

    /// `screencast.showOverlay(html, { duration })`: shown until removed, or
    /// for `duration` (then this returns after removing it).
    pub async fn show_overlay(&self, html: &str, duration: Option<Duration>) -> Result<Overlay> {
        let id = self.show(html).await?;
        if let Some(d) = duration.filter(|d| !d.is_zero()) {
            self.sleep(d).await;
            self.remove_overlay(&id).await?;
        }
        Ok(Overlay {
            page: self.page.clone(),
            id,
        })
    }

    /// `screencast.showChapter(title, { description, duration })`: a centred
    /// title card for `duration` (default 2 s), then a 300 ms fade.
    pub async fn show_chapter(
        &self,
        title: &str,
        description: Option<&str>,
        duration: Option<Duration>,
    ) -> Result<()> {
        let id = self.show(&chapter_html(title, description)).await?;
        self.sleep(duration.unwrap_or(Duration::from_secs(2))).await;
        let _ = self
            .page
            .injected_call(
                None,
                "(i, id) => { const o = i.getUserOverlay(id); const bg = o && o.querySelector('#background'); if (bg) bg.classList.add('fade-out'); }",
                &[json!(id)],
                true,
                self.page.default_timeout(),
            )
            .await;
        self.sleep(Duration::from_millis(CHAPTER_FADE_MS)).await;
        self.remove_overlay(&id).await
    }

    async fn set_visible(&self, visible: bool) -> Result<()> {
        if self
            .page
            .inner
            .screencast
            .lock()
            .unwrap()
            .overlays
            .is_empty()
        {
            return Ok(());
        }
        self.page
            .injected_call(
                None,
                "(i, v) => { i.setUserOverlaysVisible(v); }",
                &[json!(visible)],
                true,
                self.page.default_timeout(),
            )
            .await?;
        Ok(())
    }

    /// `screencast.showOverlays()`.
    pub async fn show_overlays(&self) -> Result<()> {
        self.set_visible(true).await
    }

    /// `screencast.hideOverlays()`: hidden, not removed.
    pub async fn hide_overlays(&self) -> Result<()> {
        self.set_visible(false).await
    }
}
