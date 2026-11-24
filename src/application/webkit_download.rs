//! Downloads on the WebKit engine (`page.on('download')`): the shared
//! download state, with WebKit's page and `Playwright.cancelDownload`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use serde_json::json;

use crate::application::firefox_download::Download as Shared;
use crate::application::services::Services;
use crate::application::webkit::{Page, PageInner};
use crate::domain::error::Result;
use crate::ports::WebKitSession;

/// A download of a WebKit page. Cheap to clone.
#[derive(Clone)]
pub struct Download {
    shared: Shared,
    page: Weak<PageInner>,
    uuid: String,
    wk: Arc<dyn WebKitSession>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Download")
            .field("url", &self.shared.url())
            .field("suggested_filename", &self.shared.suggested_filename())
            .finish()
    }
}

impl Download {
    pub(crate) fn new(
        page: &Page,
        uuid: String,
        url: String,
        suggested_filename: String,
        services: Services,
    ) -> Self {
        Download {
            shared: Shared::new(url, suggested_filename, services, Weak::new()),
            page: Arc::downgrade(&page.inner),
            uuid,
            wk: page.inner.wk.clone(),
        }
    }

    /// Records how the download ended (`Playwright.downloadFinished`).
    pub(crate) fn finish(&self, outcome: std::result::Result<PathBuf, String>) {
        self.shared.finish(outcome)
    }

    /// `download.url()`.
    pub fn url(&self) -> &str {
        self.shared.url()
    }
    /// `download.suggestedFilename()`.
    pub fn suggested_filename(&self) -> &str {
        self.shared.suggested_filename()
    }
    /// `download.failure()`: waits for the end; `None` when it succeeded.
    pub async fn failure(&self) -> Result<Option<String>> {
        self.shared.failure().await
    }
    /// `download.path()`: waits for the downloaded file.
    pub async fn path(&self) -> Result<PathBuf> {
        self.shared.path().await
    }
    /// The downloaded bytes (waits for the file).
    pub async fn read(&self) -> Result<Vec<u8>> {
        self.shared.read().await
    }
    /// `download.page()`: the page the download started from.
    pub fn page(&self) -> Option<Page> {
        self.page.upgrade().map(|inner| Page { inner })
    }
    /// `download.createReadStream()`: waits for the download, then reads it
    /// as a [`futures::io::AsyncRead`].
    pub async fn create_read_stream(
        &self,
    ) -> Result<impl futures::io::AsyncRead + Unpin + Send + 'static> {
        self.shared.create_read_stream().await
    }
    /// `download.delete()`: deletes the downloaded file (waits for it).
    pub async fn delete(&self) -> Result<()> {
        self.shared.delete().await
    }
    /// `download.cancel()` (`Playwright.cancelDownload`; a finished
    /// download stays as it is, as in Playwright).
    pub async fn cancel(&self) -> Result<()> {
        let _ = self
            .wk
            .call(
                None,
                "Playwright.cancelDownload",
                json!({"uuid": self.uuid}),
            )
            .await;
        Ok(())
    }
    /// `download.saveAs(path)`: copies the file (waits for it).
    pub async fn save_as(&self, path: impl AsRef<Path>) -> Result<()> {
        self.shared.save_as(path).await
    }
}
