//! Downloads (Playwright's `Download`), driven by the browser's
//! `Browser.downloadWillBegin` / `Browser.downloadProgress` events.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::events::{EventWaiter, PageEvent};
use crate::application::page::Page;
use crate::application::shared::Shared;
use crate::domain::error::{Error, Result};

/// The message Playwright gives when downloads are not accepted.
pub const DOWNLOADS_NOT_ACCEPTED: &str =
    "Pass { acceptDownloads: true } when you are creating your browser context.";

/// A file download started by a page. Cheap to clone.
#[derive(Clone)]
pub struct Download {
    inner: Arc<DownloadInner>,
}

struct DownloadInner {
    page: Page,
    guid: String,
    url: String,
    suggested_filename: String,
    path: PathBuf,
    unaccessible: Option<&'static str>,
    /// `None` while in progress; then `Ok` or the failure text.
    outcome: Mutex<Option<std::result::Result<(), String>>>,
    changed: watch::Sender<u64>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("url", &self.inner.url)
            .field("suggested_filename", &self.inner.suggested_filename)
            .finish()
    }
}

impl Download {
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    /// The file name the browser suggests (from `Content-Disposition` or the URL).
    pub fn suggested_filename(&self) -> &str {
        &self.inner.suggested_filename
    }

    /// The page that started the download.
    pub fn page(&self) -> Page {
        self.inner.page.clone()
    }

    async fn finished(&self) -> std::result::Result<(), String> {
        let mut rx = self.inner.changed.subscribe();
        loop {
            if let Some(outcome) = self.inner.outcome.lock().unwrap().clone() {
                return outcome;
            }
            if rx.changed().await.is_err() {
                return Err("download was not finished".into());
            }
        }
    }

    /// Waits for the download; `None` on success, else the failure text.
    pub async fn failure(&self) -> Result<Option<String>> {
        let outcome = self.finished().await;
        if let Some(msg) = self.inner.unaccessible {
            return Ok(Some(msg.to_owned()));
        }
        Ok(outcome.err())
    }

    /// Waits for the download and returns where it was saved.
    pub async fn path(&self) -> Result<PathBuf> {
        if let Some(msg) = self.inner.unaccessible {
            return Err(Error::InvalidArgument(format!("download.path: {msg}")));
        }
        self.finished()
            .await
            .map_err(|e| Error::InvalidArgument(format!("download.path: {e}")))?;
        Ok(self.inner.path.clone())
    }

    /// Copies the finished download to `path` (creating parent directories).
    pub async fn save_as(&self, path: impl AsRef<Path>) -> Result<()> {
        if let Some(msg) = self.inner.unaccessible {
            return Err(Error::InvalidArgument(format!("download.saveAs: {msg}")));
        }
        self.finished()
            .await
            .map_err(|e| Error::InvalidArgument(format!("download.saveAs: {e}")))?;
        let fs = &self.inner.page.inner.shared.services.fs;
        let data = fs.read(&self.inner.path)?;
        fs.write(path.as_ref(), &data)?;
        Ok(())
    }

    /// Waits for the download and returns its bytes (`createReadStream`).
    pub async fn read(&self) -> Result<Vec<u8>> {
        let path = self.path().await?;
        Ok(self.inner.page.inner.shared.services.fs.read(&path)?)
    }

    /// Cancels an unfinished download.
    pub async fn cancel(&self) -> Result<()> {
        let context = self.inner.page.context_id();
        let mut params = json!({"guid": self.inner.guid});
        if let Some(id) = context.filter(|id| !id.is_empty()) {
            params["browserContextId"] = json!(id);
        }
        self.inner
            .page
            .inner
            .shared
            .cdp
            .call(None, "Browser.cancelDownload", params)
            .await?;
        Ok(())
    }

    /// Deletes the downloaded file.
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

    fn settle(&self, outcome: std::result::Result<(), String>) {
        *self.inner.outcome.lock().unwrap() = Some(outcome);
        self.inner.changed.send_modify(|v| *v += 1);
    }
}

impl Shared {
    /// The browser-wide downloads directory, created on first use.
    pub(crate) fn downloads_dir(&self) -> Result<PathBuf> {
        let mut dir = self.downloads_dir.lock().unwrap();
        if let Some(d) = dir.as_ref() {
            return Ok(d.clone());
        }
        let d = self
            .services
            .fs
            .create_temp_dir("chromiumoxide-downloads-")?;
        *dir = Some(d.clone());
        Ok(d)
    }

    pub(crate) fn page_by_session(&self, session_id: &str) -> Option<Page> {
        let contexts: Vec<_> = self.contexts.lock().unwrap().values().cloned().collect();
        contexts
            .iter()
            .flat_map(|c| c.pages.lock().unwrap().clone())
            .find(|p| p.has_session(session_id))
    }

    fn page_with_frame(&self, frame_id: &str) -> Option<Page> {
        let contexts: Vec<_> = self.contexts.lock().unwrap().values().cloned().collect();
        contexts
            .iter()
            .flat_map(|c| c.pages.lock().unwrap().clone())
            .find(|p| p.has_frame(frame_id))
    }

    /// `Browser.downloadWillBegin` / `Browser.downloadProgress`.
    pub(crate) fn handle_download_event(&self, method: &str, p: &Value) {
        let s = |k: &str| {
            p.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match method {
            "Browser.downloadWillBegin" => {
                let Some(page) = self.page_with_frame(&s("frameId")) else {
                    return;
                };
                let Ok(dir) = self.downloads_dir() else {
                    return;
                };
                let accept = page.inner.options.accept_downloads;
                let guid = s("guid");
                let (changed, _) = watch::channel(0);
                let download = Download {
                    inner: Arc::new(DownloadInner {
                        page: page.clone(),
                        path: dir.join(&guid),
                        guid: guid.clone(),
                        url: s("url"),
                        suggested_filename: s("suggestedFilename"),
                        unaccessible: (!accept).then_some(DOWNLOADS_NOT_ACCEPTED),
                        outcome: Mutex::new(None),
                        changed,
                    }),
                };
                self.downloads
                    .lock()
                    .unwrap()
                    .insert(guid, download.clone());
                page.emit(PageEvent::Download(download));
            }
            "Browser.downloadProgress" => {
                let outcome = match p.get("state").and_then(Value::as_str) {
                    Some("completed") => Ok(()),
                    Some("canceled") => Err("canceled".to_owned()),
                    _ => return,
                };
                if let Some(d) = self.downloads.lock().unwrap().remove(&s("guid")) {
                    d.settle(outcome);
                }
            }
            _ => {}
        }
    }

    /// Settles downloads still running when the browser goes away.
    pub(crate) fn close_downloads(&self) {
        let pending: Vec<Download> = self
            .downloads
            .lock()
            .unwrap()
            .drain()
            .map(|(_, d)| d)
            .collect();
        for d in pending {
            d.settle(Err(
                "Download was canceled because the browser closed".into()
            ));
        }
    }
}

impl Page {
    /// `page.waitForEvent('download')`.
    pub fn wait_for_download(&self) -> EventWaiter<'_, Download> {
        self.wait_for_event(|e| match e {
            PageEvent::Download(d) => Some(d.clone()),
            _ => None,
        })
    }
}

impl Page {
    pub(crate) fn has_frame(&self, frame_id: &str) -> bool {
        self.inner
            .state
            .lock()
            .unwrap()
            .tree
            .frames()
            .any(|f| f.id == frame_id)
    }

    pub(crate) fn context_id(&self) -> Option<String> {
        self.inner.context.upgrade().map(|c| c.id.clone())
    }
}
