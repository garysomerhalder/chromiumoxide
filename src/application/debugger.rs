//! `context.debugger` and `page.pause()`: pausing before actions (Playwright's
//! `Debugger`), driven from another task.

use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::context::BrowserContext;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};

/// What the debugger is paused on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PausedDetails {
    /// Playwright's call title, e.g. `Click locator('button')`.
    pub title: String,
}

pub(crate) struct DebuggerState {
    pause_next: bool,
    paused: watch::Sender<Option<PausedDetails>>,
}

impl Default for DebuggerState {
    fn default() -> Self {
        Self {
            pause_next: false,
            paused: watch::channel(None).0,
        }
    }
}

/// `context.debugger`. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Debugger {
    context: BrowserContext,
}

impl BrowserContext {
    /// `context.debugger`.
    pub fn debugger(&self) -> Debugger {
        Debugger {
            context: self.clone(),
        }
    }
}

impl Debugger {
    fn with<T>(&self, f: impl FnOnce(&mut DebuggerState) -> T) -> T {
        f(&mut self.context.inner.debugger.lock().unwrap())
    }

    /// `debugger.pausedDetails()`.
    pub fn paused_details(&self) -> Option<PausedDetails> {
        self.with(|d| d.paused.borrow().clone())
    }

    /// `pausedstatechanged` events: `true` when it pauses, `false` on resume.
    pub fn paused_state_changes(&self) -> BoxStream<'static, bool> {
        let mut rx = self.with(|d| d.paused.subscribe());
        rx.mark_unchanged();
        futures::stream::unfold(rx, |mut rx| async move {
            rx.changed().await.ok()?;
            let paused = rx.borrow_and_update().is_some();
            Some((paused, rx))
        })
        .boxed()
    }

    /// `debugger.requestPause()`: pause before the next action.
    pub async fn request_pause(&self) -> Result<()> {
        self.with(|d| {
            if d.paused.borrow().is_some() {
                return Err(Error::InvalidArgument("Debugger is already paused".into()));
            }
            d.pause_next = true;
            Ok(())
        })
    }

    fn resume_with(&self, pause_next: bool) -> Result<()> {
        self.with(|d| {
            if d.paused.borrow().is_none() {
                return Err(Error::InvalidArgument("Debugger is not paused".into()));
            }
            d.pause_next = pause_next;
            d.paused.send_replace(None);
            Ok(())
        })
    }

    /// `debugger.resume()`.
    pub async fn resume(&self) -> Result<()> {
        self.resume_with(false)
    }

    /// `debugger.next()`: resume, then pause before the next action.
    pub async fn next(&self) -> Result<()> {
        self.resume_with(true)
    }

    /// Pauses with `title` until resumed.
    async fn pause(&self, title: String) {
        let mut rx = self.with(|d| {
            d.paused.send_replace(Some(PausedDetails { title }));
            d.paused.subscribe()
        });
        while rx.borrow_and_update().is_some() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Page {
    /// `page.pause()`: stops here until `context.debugger().resume()`.
    pub async fn pause(&self) -> Result<()> {
        if let Some(ctx) = self.context() {
            ctx.debugger().pause("Pause".into()).await;
        }
        Ok(())
    }

    /// `page.requestGC()`: a full garbage collection in the page.
    pub async fn request_gc(&self) -> Result<()> {
        self.send("HeapProfiler.collectGarbage", json!({})).await?;
        Ok(())
    }

    /// Before an API call: pauses when the debugger asked for it.
    pub(crate) async fn debugger_checkpoint(&self, method: &str, params: &Value) {
        let Some(ctx) = self.context() else {
            return;
        };
        let pause = {
            let mut d = ctx.inner.debugger.lock().unwrap();
            std::mem::take(&mut d.pause_next)
        };
        if !pause {
            return;
        }
        let locator = match params.get("selector").and_then(Value::as_str) {
            Some(selector) => self
                .injected_call(
                    None,
                    "(i, selector) => i.utils.asLocator('javascript', selector)",
                    &[json!(selector)],
                    true,
                    self.default_timeout(),
                )
                .await
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned)),
            None => None,
        };
        let title = crate::domain::trace::action_title(method, params, locator.as_deref());
        ctx.debugger().pause(title).await;
    }
}
