//! File choosers (Playwright's `FileChooser`): intercepted file dialogs.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::events::{EventWaiter, PageEvent};
use crate::application::handle::JsHandle;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};

/// A file dialog the page tried to open.
#[derive(Clone)]
pub struct FileChooser {
    page: Page,
    backend_node_id: i64,
    multiple: bool,
}

impl std::fmt::Debug for FileChooser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileChooser")
            .field("multiple", &self.multiple)
            .finish()
    }
}

impl FileChooser {
    pub fn page(&self) -> Page {
        self.page.clone()
    }

    /// Whether the input accepts several files.
    pub fn is_multiple(&self) -> bool {
        self.multiple
    }

    /// The `<input type=file>` element.
    pub async fn element(&self) -> Result<JsHandle> {
        let r = self
            .page
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": self.backend_node_id}),
            )
            .await?;
        let object = r
            .get("object")
            .ok_or_else(|| Error::Evaluation("file input is gone".into()))?;
        Ok(JsHandle::from_remote(&self.page, object))
    }

    /// `fileChooser.setFiles(files)`.
    pub async fn set_files(&self, files: &[PathBuf]) -> Result<()> {
        if files.len() > 1 && !self.multiple {
            return Err(Error::InvalidArgument(
                "Non-multiple file input can only accept single file".into(),
            ));
        }
        let files: Vec<String> = files
            .iter()
            .map(|f| {
                std::path::absolute(f)
                    .unwrap_or_else(|_| f.clone())
                    .display()
                    .to_string()
            })
            .collect();
        self.page
            .send(
                "DOM.setFileInputFiles",
                json!({"files": files, "backendNodeId": self.backend_node_id}),
            )
            .await?;
        Ok(())
    }
}

impl Page {
    /// `Page.fileChooserOpened`.
    pub(crate) fn on_file_chooser_opened(&self, p: &Value) {
        let Some(backend_node_id) = p.get("backendNodeId").and_then(Value::as_i64) else {
            return;
        };
        self.emit(PageEvent::FileChooser(FileChooser {
            page: self.clone(),
            backend_node_id,
            multiple: p.get("mode").and_then(Value::as_str) == Some("selectMultiple"),
        }));
    }

    async fn set_file_chooser_interception(&self, delta: i64) -> Result<()> {
        let before = self
            .inner
            .file_chooser_waiters
            .fetch_add(delta, Ordering::SeqCst);
        let after = before + delta;
        if (before == 0) != (after == 0) {
            self.send(
                "Page.setInterceptFileChooserDialog",
                json!({"enabled": after > 0}),
            )
            .await?;
        }
        Ok(())
    }

    /// `page.waitForEvent('filechooser')`: file dialogs are intercepted
    /// while waiting.
    pub fn wait_for_file_chooser(&self) -> FileChooserWaiter<'_> {
        FileChooserWaiter {
            page: self,
            timeout: None,
        }
    }
}

/// A pending `page.waitForEvent('filechooser')`.
#[must_use = "does nothing until awaited"]
pub struct FileChooserWaiter<'a> {
    page: &'a Page,
    timeout: Option<Duration>,
}

impl<'a> FileChooserWaiter<'a> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    fn waiter(&self) -> EventWaiter<'a, FileChooser> {
        let w = self.page.wait_for_event(|e| match e {
            PageEvent::FileChooser(c) => Some(c.clone()),
            _ => None,
        });
        match self.timeout {
            Some(t) => w.timeout(t),
            None => w,
        }
    }

    /// Intercepts file dialogs, runs `action`, then waits for the chooser.
    pub async fn run<A, R>(self, action: A) -> Result<FileChooser>
    where
        A: IntoFuture<Output = Result<R>>,
    {
        self.page.set_file_chooser_interception(1).await?;
        let result = self.waiter().run(action).await;
        let _ = self.page.set_file_chooser_interception(-1).await;
        result
    }
}

impl<'a> IntoFuture for FileChooserWaiter<'a> {
    type Output = Result<FileChooser>;
    type IntoFuture = BoxFuture<'a, Result<FileChooser>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            self.page.set_file_chooser_interception(1).await?;
            let result = self.waiter().await;
            let _ = self.page.set_file_chooser_interception(-1).await;
            result
        }
        .boxed()
    }
}
