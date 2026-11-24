//! Downloads on the Firefox engine: `PageEvent::Download`, finished by
//! BiDi `browsingContext.downloadEnd`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::watch;

use crate::application::services::Services;
use crate::domain::error::{Error, Result};

/// How a download ended: its file, or why it failed.
type Outcome = Option<std::result::Result<PathBuf, String>>;

/// `page.on('download')` on Firefox.
#[derive(Clone)]
pub struct Download {
    inner: Arc<DownloadInner>,
}

struct DownloadInner {
    page: std::sync::Weak<crate::application::firefox::PageInner>,
    url: String,
    suggested_filename: String,
    services: Services,
    done: watch::Sender<Outcome>,
}

impl std::fmt::Debug for Download {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Download")
            .field("url", &self.inner.url)
            .field("suggested_filename", &self.inner.suggested_filename)
            .finish()
    }
}

impl Download {
    pub(crate) fn new(
        url: String,
        suggested_filename: String,
        services: Services,
        page: std::sync::Weak<crate::application::firefox::PageInner>,
    ) -> Self {
        Download {
            inner: Arc::new(DownloadInner {
                page,
                url,
                suggested_filename,
                services,
                done: watch::channel(None).0,
            }),
        }
    }

    /// Records how the download ended (BiDi `downloadEnd`).
    pub(crate) fn finish(&self, outcome: std::result::Result<PathBuf, String>) {
        self.inner.done.send_replace(Some(outcome));
    }

    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn suggested_filename(&self) -> &str {
        &self.inner.suggested_filename
    }

    async fn outcome(&self) -> std::result::Result<PathBuf, String> {
        let mut rx = self.inner.done.subscribe();
        loop {
            if let Some(outcome) = rx.borrow_and_update().clone() {
                return outcome;
            }
            if rx.changed().await.is_err() {
                return Err("download was not finished".into());
            }
        }
    }

    /// `download.failure()`: waits for the end; `None` when it succeeded.
    pub async fn failure(&self) -> Result<Option<String>> {
        Ok(self.outcome().await.err())
    }

    /// `download.path()`: waits for the downloaded file.
    pub async fn path(&self) -> Result<PathBuf> {
        self.outcome().await.map_err(Error::Request)
    }

    /// The downloaded bytes (waits for the file).
    pub async fn read(&self) -> Result<Vec<u8>> {
        let path = self.path().await?;
        Ok(self.inner.services.fs.read(&path)?)
    }

    /// `download.page()`: the page the download started from.
    pub fn page(&self) -> Option<crate::application::firefox::Page> {
        self.inner
            .page
            .upgrade()
            .map(|inner| crate::application::firefox::Page { inner })
    }

    /// `download.createReadStream()`: waits for the download, then reads it
    /// as a [`futures::io::AsyncRead`].
    pub async fn create_read_stream(
        &self,
    ) -> Result<impl futures::io::AsyncRead + Unpin + Send + 'static> {
        Ok(futures::io::Cursor::new(self.read().await?))
    }

    /// `download.delete()`: deletes the downloaded file (waits for it).
    pub async fn delete(&self) -> Result<()> {
        if let Ok(path) = self.path().await {
            self.inner.services.fs.remove_file(&path)?;
        }
        Ok(())
    }

    /// `download.cancel()`: WebDriver BiDi has no command to cancel a
    /// download in Firefox.
    pub async fn cancel(&self) -> Result<()> {
        Err(Error::InvalidArgument(
            "download.cancel() is not available in Firefox: WebDriver BiDi has no command to \
             cancel a download"
                .into(),
        ))
    }

    /// `download.saveAs(path)`: copies the file (waits for it).
    pub async fn save_as(&self, path: impl AsRef<Path>) -> Result<()> {
        let bytes = self.read().await?;
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            self.inner.services.fs.create_dir_all(dir)?;
        }
        self.inner.services.fs.write(path, &bytes)?;
        Ok(())
    }
}
