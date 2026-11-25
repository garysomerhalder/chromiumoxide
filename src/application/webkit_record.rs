//! Video recording on the WebKit engine (`recordVideo`): WebKit's
//! screencast frames (`Screencast.startScreencast` on the page proxy, as
//! Playwright's `WKPage.startScreencast`) encoded at the video's frame rate
//! into one `.webm` per page.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::video::WHITE_JPEG;
use crate::application::webkit::Page;
use crate::application::webkit_events::PageEvent;
use crate::domain::error::{Error, Result};
use crate::domain::video::{FPS, FrameTimeline, video_size};

/// The video of a WebKit page. Cheap to clone.
#[derive(Clone)]
pub struct Video {
    inner: Arc<VideoInner>,
}

struct VideoInner {
    path: PathBuf,
    page: Page,
    done: watch::Sender<Option<std::result::Result<(), String>>>,
}

impl std::fmt::Debug for Video {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Video")
            .field("path", &self.inner.path)
            .finish()
    }
}

impl Video {
    async fn finished(&self) -> Result<()> {
        let mut rx = self.inner.done.subscribe();
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
        Ok(self.inner.path.clone())
    }

    /// Copies the finished video to `path`.
    pub async fn save_as(&self, path: impl AsRef<Path>) -> Result<()> {
        self.finished().await?;
        let fs = &self.inner.page.inner.services.fs;
        let data = fs.read(&self.inner.path)?;
        fs.write(path.as_ref(), &data)?;
        Ok(())
    }

    /// Deletes the video file (after recording ends).
    pub async fn delete(&self) -> Result<()> {
        let _ = self.finished().await;
        self.inner
            .page
            .inner
            .services
            .fs
            .remove_file(&self.inner.path)?;
        Ok(())
    }
}

impl Page {
    /// `page.video()`: the recording, when the context records video.
    pub fn video(&self) -> Option<Video> {
        self.inner.video.lock().unwrap().clone()
    }

    /// Starts recording if the context asks for it (once the first target
    /// is set up).
    pub(crate) fn start_video(&self) {
        let Some(options) = self.inner.options.record_video.clone() else {
            return;
        };
        let viewport = self.effective_viewport();
        let (w, h) = video_size(Some(viewport), options.size);
        let fps = options.fps.unwrap_or(FPS);
        let path = options
            .dir
            .join(format!("{}.webm", self.inner.proxy.to_lowercase()));
        let video = Video {
            inner: Arc::new(VideoInner {
                path: path.clone(),
                page: self.clone(),
                done: watch::channel(None).0,
            }),
        };
        *self.inner.video.lock().unwrap() = Some(video.clone());
        let page = self.clone();
        let mut wk_events = page.inner.wk.subscribe();
        let mut page_events = page.events();
        tokio::spawn(async move {
            let services = page.inner.services.clone();
            let clock = services.clock.clone();
            let outcome = async {
                let _ = services.fs.create_dir_all(&options.dir);
                let mut sink = services
                    .video
                    .start_scaled(&path, w, h, fps, (viewport.width, viewport.height))
                    .await
                    .map_err(|e| format!("cannot start video encoder: {e}"))?;
                let generation = page
                    .proxy_call(
                        "Screencast.startScreencast",
                        json!({"quality": 90, "width": w, "height": h, "toolbarHeight": 0}),
                    )
                    .await
                    .ok()
                    .and_then(|r| r.get("generation").cloned())
                    .unwrap_or(Value::Null);
                let mut timeline = FrameTimeline::new(clock.now_ms(), fps);
                let mut last: Option<Vec<u8>> = None;
                loop {
                    tokio::select! {
                        e = wk_events.next() => {
                            let Some(e) = e else { break };
                            if e.session_id.as_deref() != Some(page.inner.proxy.as_str())
                                || e.method != "Screencast.screencastFrame"
                            {
                                continue;
                            }
                            let ack = {
                                let page = page.clone();
                                let generation = generation.clone();
                                async move {
                                    let _ = page
                                        .proxy_call(
                                            "Screencast.screencastFrameAck",
                                            json!({"generation": generation}),
                                        )
                                        .await;
                                }
                            };
                            tokio::spawn(ack);
                            let data = e.params["data"].as_str().unwrap_or_default();
                            let Ok(frame) = base64::engine::general_purpose::STANDARD.decode(data) else {
                                continue;
                            };
                            let now = clock.now_ms();
                            if let Some(prev) = &last {
                                for _ in 0..timeline.frames_before(now) {
                                    sink.write_frame(prev).await.map_err(|e| e.to_string())?;
                                }
                            }
                            timeline.push(now);
                            last = Some(frame);
                        }
                        e = page_events.next() => {
                            if matches!(e, None | Some(PageEvent::Close)) {
                                break;
                            }
                        }
                    }
                }
                let frame = last.unwrap_or_else(|| {
                    base64::engine::general_purpose::STANDARD
                        .decode(WHITE_JPEG)
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
            video.inner.done.send_replace(Some(outcome));
        });
    }
}
