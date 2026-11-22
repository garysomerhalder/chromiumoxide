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

/// Picking mode in one frame: hover highlights the element with its
/// locator, a click picks it (and does not reach the page). Re-installed by
/// every poll, so it survives new documents.
const PICK_TICK: &str = "(i) => {
  let state = window.__coxPick;
  if (!state) {
    const h = i._ensureHighlight();
    let last = null;
    const target = (e) => { const el = e.composedPath ? e.composedPath()[0] : e.target; return el instanceof Element ? el : null; };
    const move = (e) => {
      const el = target(e);
      if (!el || el === last) return;
      last = el;
      h.updateHighlight([{ element: el, color: 'rgba(111, 168, 220, .498)', tooltipText: i.utils.asLocator('javascript', i.generateSelectorSimple(el)) }]);
    };
    const swallow = (e) => { e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation(); };
    const click = (e) => { swallow(e); const el = target(e); if (el) state.picked = i.generateSelectorSimple(el); };
    const kinds = ['mousedown', 'mouseup', 'pointerdown', 'pointerup', 'auxclick', 'dblclick', 'contextmenu'];
    const stop = () => {
      removeEventListener('mousemove', move, true);
      removeEventListener('click', click, true);
      for (const k of kinds) removeEventListener(k, swallow, true);
      h.updateHighlight([]);
      delete window.__coxPick;
    };
    state = { picked: null, stop };
    addEventListener('mousemove', move, true);
    addEventListener('click', click, true);
    for (const k of kinds) addEventListener(k, swallow, true);
    window.__coxPick = state;
  }
  const picked = state.picked;
  state.picked = null;
  return picked;
}";

const PICK_STOP: &str = "() => { if (window.__coxPick) window.__coxPick.stop(); }";

impl Page {
    async fn stop_picking(&self) {
        for frame in self.frames() {
            let _ = self
                .call_function_in(
                    Some(frame.id()),
                    crate::domain::frame::World::Utility,
                    &format!("function() {{ ({PICK_STOP})(); }}"),
                    &[],
                    self.default_timeout(),
                    "stopping locator picking",
                )
                .await;
        }
    }

    /// The page-level selector of `selector` inside frame `frame_id`.
    async fn selector_from_page(&self, frame_id: &str, selector: String) -> Result<String> {
        let mut selector = selector;
        let mut id = frame_id.to_owned();
        while let Some(parent) = self.frame_by_id(&id).parent_frame() {
            let parent_id = (!parent.is_main()).then(|| parent.id().to_owned());
            let owner = self.frame_owner_selector(&id, parent_id.as_deref()).await?;
            selector = format!("{owner} >> internal:control=enter-frame >> {selector}");
            id = parent.id().to_owned();
        }
        Ok(selector)
    }

    /// `page.pickLocator()`: waits for the user to click an element (in any
    /// frame) and returns its locator.
    pub async fn pick_locator(&self) -> Result<crate::application::locator::Locator> {
        self.inner
            .picking
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let clock = self.inner.shared.services.clock.clone();
        loop {
            if !self.inner.picking.load(std::sync::atomic::Ordering::SeqCst) || self.is_closed() {
                self.stop_picking().await;
                return Err(Error::InvalidArgument(
                    "Locator picking was cancelled".into(),
                ));
            }
            let main_id = self.main_frame().id().to_owned();
            for frame in self.frames() {
                let frame_arg = (frame.id() != main_id).then(|| frame.id().to_owned());
                let picked = self
                    .injected_call(
                        frame_arg.as_deref(),
                        PICK_TICK,
                        &[],
                        true,
                        std::time::Duration::from_millis(500),
                    )
                    .await;
                if let Ok(Value::String(selector)) = picked {
                    self.inner
                        .picking
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    self.stop_picking().await;
                    let selector = match frame_arg {
                        Some(id) => self.selector_from_page(&id, selector).await?,
                        None => selector,
                    };
                    return Ok(self.locator(&selector));
                }
            }
            clock.sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// `page.cancelPickLocator()`: leaves picking mode (no-op otherwise).
    pub async fn cancel_pick_locator(&self) -> Result<()> {
        self.inner
            .picking
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
