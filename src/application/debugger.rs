//! `context.debugger` and `page.pause()`: pausing before actions (Playwright's
//! `Debugger`), driven from another task.

use futures::StreamExt;
use futures::stream::BoxStream;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::context::BrowserContext;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::source::SourceLocation;

/// What the debugger is paused on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PausedDetails {
    /// Playwright's call title, e.g. `Click locator('button')`.
    pub title: String,
    /// Where the call came from, when the binary has debug info (read from
    /// a backtrace: the innermost frame outside chromiumoxide and the
    /// toolchain, runtime and other dependencies).
    pub location: Option<SourceLocation>,
}

pub(crate) struct DebuggerState {
    pause_next: bool,
    pause_at: Option<SourceLocation>,
    paused: watch::Sender<Option<PausedDetails>>,
}

impl Default for DebuggerState {
    fn default() -> Self {
        Self {
            pause_next: false,
            pause_at: None,
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

    fn resume_with(&self, pause_next: bool, pause_at: Option<SourceLocation>) -> Result<()> {
        self.with(|d| {
            if d.paused.borrow().is_none() {
                return Err(Error::InvalidArgument("Debugger is not paused".into()));
            }
            d.pause_next = pause_next;
            d.pause_at = pause_at;
            d.paused.send_replace(None);
            Ok(())
        })
    }

    /// `debugger.resume()`.
    pub async fn resume(&self) -> Result<()> {
        self.resume_with(false, None)
    }

    /// `debugger.next()`: resume, then pause before the next action.
    pub async fn next(&self) -> Result<()> {
        self.resume_with(true, None)
    }

    /// `debugger.runTo(location)`: resume, then pause before the next action
    /// called from `location` (a part of the file path, and optionally the
    /// line and column). Call sites come from backtraces, so this needs a
    /// binary with debug info (the default for `cargo test` and debug builds).
    pub async fn run_to(&self, location: SourceLocation) -> Result<()> {
        self.resume_with(false, Some(location))
    }

    /// Pauses with `title` until resumed.
    async fn pause(&self, title: String, location: Option<SourceLocation>) {
        let mut rx = self.with(|d| {
            d.pause_next = false;
            d.pause_at = None;
            d.paused
                .send_replace(Some(PausedDetails { title, location }));
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
            let location = caller_location(&*self.inner.shared.services.env);
            ctx.debugger().pause("Pause".into(), location).await;
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
        let (next, at) = {
            let d = ctx.inner.debugger.lock().unwrap();
            (d.pause_next, d.pause_at.clone())
        };
        if !next && at.is_none() {
            return;
        }
        // Captured before any await, while the caller's frames are on the stack.
        let location = caller_location(&*self.inner.shared.services.env);
        let at_location = matches!((&at, &location), (Some(at), Some(here)) if here.matches(at));
        if !next && !at_location {
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
        ctx.debugger().pause(title, location).await;
    }
}

/// The innermost frame of the current stack that is the caller's code: not
/// chromiumoxide's own `src`, the Rust toolchain or a crate from a registry.
fn caller_location(env: &dyn crate::ports::Environment) -> Option<SourceLocation> {
    let own_src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    env.call_stack().into_iter().find(|loc| {
        let path = std::path::Path::new(&loc.file);
        let toolchain = [
            "/rustc/",
            "/.cargo/",
            "/.rustup/",
            "\\rustc\\",
            "\\.cargo\\",
        ]
        .iter()
        .any(|p| loc.file.contains(p));
        path.is_absolute() && !toolchain && !path.starts_with(&own_src)
    })
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
