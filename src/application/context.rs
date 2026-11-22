//! Browser contexts: isolated profiles with their own emulation.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::page::{DEFAULT_TIMEOUT, Page};
use crate::application::shared::Shared;
use crate::application::wait::wait_until;
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};

pub(crate) struct ContextInner {
    pub(crate) id: String,
    pub(crate) options: Arc<ContextOptions>,
    pub(crate) shared: Arc<Shared>,
    pub(crate) pages: Mutex<Vec<Page>>,
    pub(crate) init_errors: Mutex<HashMap<String, String>>,
    pub(crate) changed: watch::Sender<u64>,
    pub(crate) closed: AtomicBool,
}

impl ContextInner {
    pub(crate) fn add_page(&self, page: Page) {
        self.pages.lock().unwrap().push(page);
        self.changed.send_modify(|v| *v += 1);
    }

    pub(crate) fn page_failed(&self, target_id: &str, error: &Error) {
        self.init_errors
            .lock()
            .unwrap()
            .insert(target_id.to_owned(), error.to_string());
        self.changed.send_modify(|v| *v += 1);
    }

    pub(crate) fn forget_page(&self, target_id: &str) {
        self.pages
            .lock()
            .unwrap()
            .retain(|p| p.target_id() != target_id);
        self.changed.send_modify(|v| *v += 1);
    }
}

/// An isolated browser profile (Playwright's `BrowserContext`). Cheap to clone.
#[derive(Clone)]
pub struct BrowserContext {
    pub(crate) inner: Arc<ContextInner>,
}

impl std::fmt::Debug for BrowserContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserContext")
            .field("id", &self.inner.id)
            .finish()
    }
}

impl BrowserContext {
    /// `Target.createBrowserContext` and registration.
    pub(crate) async fn create(
        shared: Arc<Shared>,
        options: ContextOptions,
    ) -> Result<BrowserContext> {
        options.validate()?;
        let r = shared
            .cdp
            .call(
                None,
                "Target.createBrowserContext",
                json!({"disposeOnDetach": true}),
            )
            .await?;
        let id = r
            .get("browserContextId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Serde("createBrowserContext returned no id".into()))?
            .to_owned();
        let (changed, _) = watch::channel(0);
        let inner = Arc::new(ContextInner {
            id: id.clone(),
            options: Arc::new(options),
            shared: shared.clone(),
            pages: Mutex::new(Vec::new()),
            init_errors: Mutex::new(HashMap::new()),
            changed,
            closed: AtomicBool::new(false),
        });
        shared.contexts.lock().unwrap().insert(id, inner.clone());
        if !inner.options.permissions.is_empty() {
            shared
                .cdp
                .call(
                    None,
                    "Browser.grantPermissions",
                    json!({"browserContextId": inner.id, "permissions": inner.options.permissions}),
                )
                .await?;
        }
        Ok(BrowserContext { inner })
    }

    /// The protocol id of this context.
    pub fn id(&self) -> &str {
        &self.inner.id
    }

    /// The options this context was created with.
    pub fn options(&self) -> &ContextOptions {
        &self.inner.options
    }

    /// Opens a new page in this context.
    pub async fn new_page(&self) -> Result<Page> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("context has been closed".into()));
        }
        let shared = &self.inner.shared;
        let r = shared
            .cdp
            .call(
                None,
                "Target.createTarget",
                json!({"url": "about:blank", "browserContextId": self.inner.id}),
            )
            .await?;
        let target_id = r
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Serde("createTarget returned no targetId".into()))?
            .to_owned();
        let inner = &self.inner;
        wait_until(
            &*shared.services.clock,
            &inner.changed,
            DEFAULT_TIMEOUT,
            "creating a page",
            || {
                if let Some(e) = inner.init_errors.lock().unwrap().remove(&target_id) {
                    return Some(Err(Error::TargetClosed(format!(
                        "page failed to initialise: {e}"
                    ))));
                }
                inner
                    .pages
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|p| p.target_id() == target_id)
                    .cloned()
                    .map(Ok)
            },
        )
        .await
    }

    /// Open pages.
    pub fn pages(&self) -> Vec<Page> {
        self.inner.pages.lock().unwrap().clone()
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Closes all pages and disposes the context.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let shared = &self.inner.shared;
        let _ = shared
            .cdp
            .call(
                None,
                "Target.disposeBrowserContext",
                json!({"browserContextId": self.inner.id}),
            )
            .await;
        let pages = std::mem::take(&mut *self.inner.pages.lock().unwrap());
        for p in pages {
            p.mark_closed();
        }
        shared.contexts.lock().unwrap().remove(&self.inner.id);
        self.inner.changed.send_modify(|v| *v += 1);
        Ok(())
    }
}
