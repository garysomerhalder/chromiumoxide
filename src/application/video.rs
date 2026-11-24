//! Video recording (`recordVideo`): the page's screencast frames are encoded
//! at 25 fps into one `.webm` per page.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::events::PageEvent;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::video::{FPS, FrameTimeline, video_size};

/// A 16x16 white JPEG (made by Chromium), used when a page produced no frame.
pub(crate) const WHITE_JPEG: &str = "/9j/4AAQSkZJRgABAQAAAQABAAD/4gHYSUNDX1BST0ZJTEUAAQEAAAHIAAAAAAQwAABtbnRyUkdCIFhZWiAH4AABAAEAAAAAAABhY3NwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAQAA9tYAAQAAAADTLQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAlkZXNjAAAA8AAAACRyWFlaAAABFAAAABRnWFlaAAABKAAAABRiWFlaAAABPAAAABR3dHB0AAABUAAAABRyVFJDAAABZAAAAChnVFJDAAABZAAAAChiVFJDAAABZAAAAChjcHJ0AAABjAAAADxtbHVjAAAAAAAAAAEAAAAMZW5VUwAAAAgAAAAcAHMAUgBHAEJYWVogAAAAAAAAb6IAADj1AAADkFhZWiAAAAAAAABimQAAt4UAABjaWFlaIAAAAAAAACSgAAAPhAAAts9YWVogAAAAAAAA9tYAAQAAAADTLXBhcmEAAAAAAAQAAAACZmYAAPKnAAANWQAAE9AAAApbAAAAAAAAAABtbHVjAAAAAAAAAAEAAAAMZW5VUwAAACAAAAAcAEcAbwBvAGcAbABlACAASQBuAGMALgAgADIAMAAxADb/2wBDAAYEBQYFBAYGBQYHBwYIChAKCgkJChQODwwQFxQYGBcUFhYaHSUfGhsjHBYWICwgIyYnKSopGR8tMC0oMCUoKSj/2wBDAQcHBwoIChMKChMoGhYaKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCgoKCj/wAARCAAQABADASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAj/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFAEBAAAAAAAAAAAAAAAAAAAAAP/EABQRAQAAAAAAAAAAAAAAAAAAAAD/2gAMAwEAAhEDEQA/AKpAB//Z";

/// The video of a page (Playwright's `Video`). Cheap to clone.
#[derive(Clone)]
pub struct Video {
    inner: Arc<VideoInner>,
}

struct VideoInner {
    path: PathBuf,
    page: Page,
    /// `None` while recording; then the outcome.
    done: watch::Sender<Option<std::result::Result<(), String>>>,
}

impl std::fmt::Debug for Video {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Video")
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
        let fs = &self.inner.page.inner.shared.services.fs;
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
            .shared
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

    /// Starts the screencast-to-video recorder if the context asks for it.
    pub(crate) fn start_video(&self) {
        let Some(options) = self.inner.options.record_video.clone() else {
            return;
        };
        let (w, h) = video_size(self.inner.options.viewport, options.size);
        let fps = options.fps.unwrap_or(FPS);
        let path = options
            .dir
            .join(format!("{}.webm", self.target_id().to_lowercase()));
        let (done, _) = watch::channel(None);
        let video = Video {
            inner: Arc::new(VideoInner {
                path: path.clone(),
                page: self.clone(),
                done,
            }),
        };
        *self.inner.video.lock().unwrap() = Some(video.clone());
        let page = self.clone();
        let mut cdp_events = page.inner.shared.cdp.subscribe();
        let mut page_events = page.events();
        tokio::spawn(async move {
            let clock = page.inner.shared.services.clock.clone();
            let encoder = page.inner.shared.services.video.clone();
            let outcome = async {
                let mut sink = encoder
                    .start(&path, w, h, fps)
                    .await
                    .map_err(|e| format!("cannot start video encoder: {e}"))?;
                let _ = page
                    .send(
                        "Page.startScreencast",
                        json!({"format": "jpeg", "quality": 90, "maxWidth": w, "maxHeight": h,
                               "maxFramesInFlight": if fps > 30 { 10 } else { 5 }}),
                    )
                    .await;
                let mut timeline = FrameTimeline::new(clock.now_ms(), fps);
                let mut last: Option<Vec<u8>> = None;
                loop {
                    tokio::select! {
                        e = cdp_events.next() => {
                            let Some(e) = e else { break };
                            if e.session_id.as_deref() != Some(page.inner.session_id.as_str())
                                || e.method != "Page.screencastFrame"
                            {
                                continue;
                            }
                            if let Some(sid) = e.params.get("sessionId").cloned() {
                                let ack = page.inner.shared.cdp.call(
                                    Some(&page.inner.session_id),
                                    "Page.screencastFrameAck",
                                    json!({"sessionId": sid}),
                                );
                                tokio::spawn(ack);
                            }
                            let data = e.params.get("data").and_then(Value::as_str).unwrap_or_default();
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
