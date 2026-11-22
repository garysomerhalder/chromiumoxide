//! Locators: Playwright's auto-waiting, strict element references.
//!
//! Selector engines, actionability checks and DOM helpers run in the page via
//! Playwright's injected script (`scripts/injected.js`); retry timing, the
//! checks each action needs and click geometry come from the domain.

use std::future::{Future, IntoFuture};
use std::path::PathBuf;

use base64::Engine;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::domain::actionability::{
    Action, Point, PointError, clickable_point, required_states, retry_wait_ms,
};
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
use crate::domain::input::MouseButton;
use crate::domain::screenshot::{Rect, ScreenshotOptions, Size, Target, enclosing_int_rect};
use crate::domain::selectors::{self, Filter, RoleOptions, TextMatch, compose};

const INJECTED_SOURCE: &str = include_str!("scripts/injected.js");
const INJECTED: &str = "globalThis.__chromiumoxide_injected";

/// A resolved element in the utility world.
#[derive(Debug, Clone)]
pub(crate) struct ElementRef {
    pub(crate) object_id: String,
    /// The innermost `<iframe>` element the node lives in (None: main frame).
    pub(crate) frame_owner: Option<String>,
}

/// `locator.waitFor({ state })`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WaitForState {
    Attached,
    Detached,
    #[default]
    Visible,
    Hidden,
}

/// The element's border box in CSS pixels (main-frame viewport coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One attempt's outcome inside the retry loop.
/// Playwright's in-page part of `drop()`.
const DROP: &str = "(i, node, payloads, data, point) => {
  if (!node.isConnected || node.nodeType !== 1) return 'error:notconnected';
  const dt = new DataTransfer();
  for (const p of payloads) {
    const bytes = Uint8Array.from(atob(p.buffer), (c) => c.charCodeAt(0));
    dt.items.add(new File([bytes], p.name, { type: p.mimeType }));
  }
  for (const entry of data) dt.setData(entry.mimeType, entry.value);
  const makeEvent = (type) => new DragEvent(type, { bubbles: true, cancelable: true, composed: true,
    clientX: point.x, clientY: point.y, dataTransfer: dt });
  node.dispatchEvent(makeEvent('dragenter'));
  const over = makeEvent('dragover');
  node.dispatchEvent(over);
  if (!over.defaultPrevented) { node.dispatchEvent(makeEvent('dragleave')); return 'not-accepted'; }
  node.dispatchEvent(makeEvent('drop'));
  return 'accepted';
}";

/// An in-memory file for `locator.drop()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropFile {
    pub name: String,
    pub mime_type: String,
    pub buffer: Vec<u8>,
}

impl DropFile {
    pub fn new(name: &str, mime_type: &str, buffer: &[u8]) -> Self {
        Self {
            name: name.to_owned(),
            mime_type: mime_type.to_owned(),
            buffer: buffer.to_vec(),
        }
    }
}

/// What `locator.drop()` drops: files (paths or buffers) and/or data by MIME type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DropPayload {
    files: Vec<DropFile>,
    paths: Vec<PathBuf>,
    data: Vec<(String, String)>,
}

impl DropPayload {
    pub fn file(mut self, file: DropFile) -> Self {
        self.files.push(file);
        self
    }
    /// A file read from disk (name and MIME type from the path).
    pub fn path(mut self, path: impl Into<PathBuf>) -> Self {
        self.paths.push(path.into());
        self
    }
    /// A data entry, e.g. `("text/plain", "hello")`.
    pub fn data(mut self, mime_type: &str, value: &str) -> Self {
        self.data.push((mime_type.to_owned(), value.to_owned()));
        self
    }
}

enum Attempt<T> {
    Done(T),
    Retry(String),
}

/// A way to find element(s) on a page, re-resolved on every use (Playwright's `Locator`).
#[derive(Clone)]
pub struct Locator {
    pub(crate) page: Page,
    /// Frame the selector starts from (`None`: the main frame).
    frame: Option<String>,
    selector: String,
}

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "locator({:?})", self.selector)
    }
}

impl Page {
    /// `page.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.clone(),
            frame: None,
            selector: selector.to_owned(),
        }
    }
    pub fn get_by_role(&self, role: &str, options: RoleOptions) -> Locator {
        self.locator(&selectors::get_by_role(role, &options))
    }
    pub fn get_by_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_text(&text.into(), exact))
    }
    pub fn get_by_label(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_label(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }
    pub fn get_by_alt_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_alt_text(&text.into(), exact))
    }
    pub fn get_by_title(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_title(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id(
            &self.page_ref().test_id_attribute(),
            &id.into(),
        ))
    }

    /// The utility-world context of the main frame with the injected script installed.
    async fn injected_context(&self, frame: Option<&str>, timeout: Duration) -> Result<i64> {
        let context_id = self.frame_context(frame, World::Utility, timeout).await?;
        let installed = self
            .send(
                "Runtime.evaluate",
                json!({"expression": format!("!!{INJECTED}"), "contextId": context_id, "returnByValue": true}),
            )
            .await?;
        if installed.pointer("/result/value") != Some(&Value::Bool(true)) {
            let options = json!({
                "isUnderTest": false, "sdkLanguage": "javascript", "frameSeq": 0,
                "testIdAttributeName": self.test_id_attribute(), "stableRafCount": 1, "browserName": "chromium",
                "isUtilityWorld": true, "customEngines": self.custom_engines(),
            });
            let install = format!(
                "(() => {{ if ({INJECTED}) return; const module = {{}};\n{INJECTED_SOURCE}\n{INJECTED} = new (module.exports.InjectedScript())(globalThis, {options}); }})()"
            );
            let r = self
                .send(
                    "Runtime.evaluate",
                    json!({"expression": install, "contextId": context_id}),
                )
                .await?;
            crate::application::js::result_value(&r)?;
        }
        Ok(context_id)
    }

    /// Calls `body(injected, ...args)` in the utility world.
    pub(crate) async fn injected_call(
        &self,
        frame: Option<&str>,
        body: &str,
        args: &[Value],
        by_value: bool,
        timeout: Duration,
    ) -> Result<Value> {
        let context_id = self.injected_context(frame, timeout).await?;
        let declaration = format!("function(...args) {{ return ({body})({INJECTED}, ...args); }}");
        let arguments: Vec<Value> = args.iter().map(|v| json!({"value": v})).collect();
        let r = self
            .send(
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": declaration, "executionContextId": context_id, "arguments": arguments,
                       "returnByValue": by_value, "awaitPromise": true}),
            )
            .await?;
        if by_value {
            crate::application::js::result_value(&r)
        } else {
            if r.get("exceptionDetails").is_some() {
                crate::application::js::result_value(&r)?;
            }
            Ok(r["result"].clone())
        }
    }

    /// `querySelector` in a frame; the element's object id.
    async fn query_in(
        &self,
        frame: Option<&str>,
        selector: &str,
        strict: bool,
        timeout: Duration,
    ) -> Result<Option<String>> {
        let r = self
            .injected_call(
                frame,
                "(i, selector, strict) => i.querySelector(i.parseSelector(selector), document, strict) || null",
                &[json!(selector), json!(strict)],
                false,
                timeout,
            )
            .await?;
        Ok(r.get("objectId").and_then(Value::as_str).map(str::to_owned))
    }

    /// Calls `body(injected, element, ...args)`.
    async fn element_call(
        &self,
        el: &ElementRef,
        body: &str,
        args: &[Value],
        by_value: bool,
    ) -> Result<Value> {
        let declaration =
            format!("function(...args) {{ return ({body})({INJECTED}, this, ...args); }}");
        let arguments: Vec<Value> = args.iter().map(|v| json!({"value": v})).collect();
        let r = self
            .send(
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": declaration, "objectId": el.object_id, "arguments": arguments,
                       "returnByValue": by_value, "awaitPromise": true}),
            )
            .await?;
        if by_value {
            crate::application::js::result_value(&r)
        } else {
            if r.get("exceptionDetails").is_some() {
                crate::application::js::result_value(&r)?;
            }
            Ok(r["result"].clone())
        }
    }

    pub(crate) async fn element_screenshot_target(
        &self,
        el: &ElementRef,
        viewport: Size,
        timeout: Duration,
    ) -> Result<Target> {
        let _ = timeout;
        let bbox = self.box_of(el).await?.ok_or_else(|| {
            Error::Evaluation("Node is either not visible or not an HTMLElement".into())
        })?;
        if bbox.width == 0.0 || bbox.height == 0.0 {
            return Err(Error::Evaluation("Node has 0 width or height.".into()));
        }
        let fits_viewport = bbox.width <= viewport.width && bbox.height <= viewport.height;
        let scroll = self
            .call_function(
                World::Utility,
                "function() { return [window.scrollX, window.scrollY]; }",
                &[],
                timeout,
                "reading scroll",
            )
            .await?;
        let sx = scroll[0].as_f64().unwrap_or_default();
        let sy = scroll[1].as_f64().unwrap_or_default();
        let rect = enclosing_int_rect(Rect::new(bbox.x + sx, bbox.y + sy, bbox.width, bbox.height));
        Ok(Target::Document {
            rect,
            fits_viewport,
        })
    }

    async fn box_of(&self, el: &ElementRef) -> Result<Option<BoundingBox>> {
        let Ok(model) = self
            .send("DOM.getBoxModel", json!({"objectId": el.object_id}))
            .await
        else {
            return Ok(None);
        };
        let quad: Vec<f64> = model
            .pointer("/model/border")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        if quad.len() != 8 {
            return Ok(None);
        }
        let xs = [quad[0], quad[2], quad[4], quad[6]];
        let ys = [quad[1], quad[3], quad[5], quad[7]];
        let x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let y = ys.iter().cloned().fold(f64::INFINITY, f64::min);
        let w = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - x;
        let h = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - y;
        Ok(Some(BoundingBox {
            x,
            y,
            width: w,
            height: h,
        }))
    }
}

impl Locator {
    pub fn selector(&self) -> &str {
        &self.selector
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    fn with(&self, selector: String) -> Locator {
        Locator {
            page: self.page.clone(),
            frame: self.frame.clone(),
            selector,
        }
    }

    pub(crate) fn in_frame(page: &Page, frame: Option<String>, selector: String) -> Locator {
        Locator {
            page: page.clone(),
            frame,
            selector,
        }
    }

    /// `locator.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            frame: self.frame.clone(),
            selector: compose::chain(&self.selector, selector),
        }
    }

    /// `locator.contentFrame()`: the iframe this locator points to.
    pub fn content_frame(&self) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            frame: self.frame.clone(),
            selector: self.selector.clone(),
        }
    }

    /// `locator.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        self.with(compose::chain(&self.selector, selector))
    }
    /// `locator.locator(otherLocator)`.
    pub fn locator_of(&self, other: &Locator) -> Locator {
        self.with(compose::chain_locator(&self.selector, &other.selector))
    }
    pub fn get_by_role(&self, role: &str, options: RoleOptions) -> Locator {
        self.locator(&selectors::get_by_role(role, &options))
    }
    pub fn get_by_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_text(&text.into(), exact))
    }
    pub fn get_by_label(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_label(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id(
            &self.page_ref().test_id_attribute(),
            &id.into(),
        ))
    }
    pub fn nth(&self, index: i64) -> Locator {
        self.with(compose::nth(&self.selector, index))
    }
    pub fn first(&self) -> Locator {
        self.nth(0)
    }
    pub fn last(&self) -> Locator {
        self.nth(-1)
    }
    pub fn filter(&self, filter: Filter) -> Locator {
        self.with(compose::filter(&self.selector, &filter))
    }
    pub fn and(&self, other: &Locator) -> Locator {
        self.with(compose::and(&self.selector, &other.selector))
    }
    pub fn or(&self, other: &Locator) -> Locator {
        self.with(compose::or(&self.selector, &other.selector))
    }

    // ---- resolution -------------------------------------------------------

    /// Resolves the iframe hops in the selector; returns the frame to query
    /// in, the remaining selector and the innermost iframe element.
    pub(crate) async fn scope(
        &self,
        timeout: Duration,
    ) -> Result<Option<(Option<String>, String, Option<String>)>> {
        let parts = split_frames(&self.selector);
        let (last, hops) = parts.split_last().expect("at least one part");
        let mut frame = self.frame.clone();
        let mut owner = None;
        for hop in hops {
            let Some(el) = self
                .page
                .query_in(frame.as_deref(), hop, true, timeout)
                .await?
            else {
                return Ok(None);
            };
            let node = self
                .page
                .send("DOM.describeNode", json!({"objectId": el}))
                .await?;
            let Some(frame_id) = node.pointer("/node/frameId").and_then(Value::as_str) else {
                return Err(Error::InvalidArgument(format!(
                    "Selector \"{hop}\" did not resolve to an <iframe>"
                )));
            };
            frame = Some(frame_id.to_owned());
            owner = Some(el);
        }
        Ok(Some((frame, last.clone(), owner)))
    }

    async fn query(&self, strict: bool, timeout: Duration) -> Result<Option<ElementRef>> {
        let Some((frame, selector, mut frame_owner)) = self.scope(timeout).await? else {
            return Ok(None);
        };
        // A locator made from a child frame has no iframe hop: find the
        // frame's <iframe> element so pointer coordinates can be converted.
        if frame_owner.is_none()
            && let Some(id) = frame.as_deref()
            && self.page.main_frame_id().ok().as_deref() != Some(id)
        {
            frame_owner = self.page.frame_owner_object(id).await?;
        }
        Ok(self
            .page
            .query_in(frame.as_deref(), &selector, strict, timeout)
            .await?
            .map(|object_id| ElementRef {
                object_id,
                frame_owner,
            }))
    }

    /// Runs `attempt` with Playwright's retry schedule until it is done or
    /// the timeout elapses. Lost contexts and detached nodes are retried.
    async fn retry<T, F, Fut>(&self, what: &str, timeout: Duration, mut attempt: F) -> Result<T>
    where
        F: FnMut(usize) -> Fut,
        Fut: Future<Output = Result<Attempt<T>>>,
    {
        let clock = self.page.inner.shared.services.clock.clone();
        let start = clock.now_ms();
        let deadline = start + timeout.as_millis() as u64;
        let mut last = format!("waiting for {self:?}");
        let mut retry = 0usize;
        loop {
            self.page.run_locator_handlers().await?;
            if retry > 0 {
                let wait = retry_wait_ms(retry);
                if clock.now_ms() + wait > deadline {
                    break;
                }
                if wait > 0 {
                    clock.sleep(Duration::from_millis(wait)).await;
                }
            }
            let remaining = Duration::from_millis(deadline.saturating_sub(clock.now_ms()).max(1));
            let outcome =
                crate::application::wait::with_timeout(&*clock, remaining, what, attempt(retry))
                    .await;
            retry += 1;
            match outcome {
                Ok(Attempt::Done(v)) => return Ok(v),
                Ok(Attempt::Retry(reason)) => last = reason,
                Err(Error::Timeout { .. }) => break,
                Err(e) if crate::application::js::is_context_lost(&e) => last = e.to_string(),
                Err(Error::Protocol { message, .. })
                    if message.contains("detached") || message.contains("Could not find node") =>
                {
                    last = message
                }
                Err(e) => return Err(e),
            }
            if clock.now_ms() >= deadline {
                break;
            }
        }
        Err(Error::Timeout {
            ms: timeout.as_millis() as u64,
            action: format!("{what} {self:?}: {last}"),
        })
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.page.default_timeout()
    }

    async fn resolved(&self, timeout: Duration) -> Result<Attempt<ElementRef>> {
        Ok(match self.query(true, timeout).await? {
            Some(el) => Attempt::Done(el),
            None => Attempt::Retry(format!("waiting for {self:?}")),
        })
    }

    /// Waits for the element (strict) and returns it.
    pub(crate) async fn element(&self, what: &str, timeout: Duration) -> Result<ElementRef> {
        self.retry(what, timeout, |_| self.resolved(timeout)).await
    }

    async fn check_states(&self, el: &ElementRef, action: Action) -> Result<Option<String>> {
        let states: Vec<&str> = required_states(action).iter().map(|s| s.name()).collect();
        let r = self
            .page
            .element_call(
                el,
                "async (i, node, states) => await i.checkElementStates(node, states)",
                &[json!(states)],
                true,
            )
            .await?;
        Ok(match r {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(format!(
                "element is not {}",
                other
                    .get("missingState")
                    .and_then(Value::as_str)
                    .unwrap_or("ready")
            )),
        })
    }

    /// Scroll, find the clickable point and verify the hit target.
    async fn pointer_point(
        &self,
        el: &ElementRef,
        force: bool,
        position: Option<Point>,
    ) -> Result<std::result::Result<Point, String>> {
        if let Err(e) = self
            .page
            .send(
                "DOM.scrollIntoViewIfNeeded",
                json!({"objectId": el.object_id}),
            )
            .await
        {
            return Ok(Err(e.to_string()));
        }
        let point = match position {
            Some(offset) => {
                let Some(b) = self.page.box_of(el).await? else {
                    return Ok(Err("element is not visible".into()));
                };
                Point {
                    x: b.x + offset.x,
                    y: b.y + offset.y,
                }
            }
            None => {
                let quads = self
                    .page
                    .send("DOM.getContentQuads", json!({"objectId": el.object_id}))
                    .await;
                let Ok(quads) = quads else {
                    return Ok(Err("element is not visible".into()));
                };
                let quads: Vec<[Point; 4]> = quads["quads"]
                    .as_array()
                    .map(|qs| {
                        qs.iter()
                            .filter_map(|q| {
                                let v: Vec<f64> =
                                    q.as_array()?.iter().filter_map(Value::as_f64).collect();
                                (v.len() == 8).then(|| {
                                    [
                                        Point { x: v[0], y: v[1] },
                                        Point { x: v[2], y: v[3] },
                                        Point { x: v[4], y: v[5] },
                                        Point { x: v[6], y: v[7] },
                                    ]
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let vp = self
                    .page
                    .call_function(
                        World::Utility,
                        "function() { return [innerWidth, innerHeight]; }",
                        &[],
                        self.timeout(),
                        "reading viewport",
                    )
                    .await?;
                match clickable_point(
                    &quads,
                    vp[0].as_f64().unwrap_or_default(),
                    vp[1].as_f64().unwrap_or_default(),
                ) {
                    Ok(p) => p,
                    Err(PointError::NotVisible) => return Ok(Err("element is not visible".into())),
                    Err(PointError::NotInViewport) => {
                        return Ok(Err("element is outside of the viewport".into()));
                    }
                }
            }
        };
        if !force {
            // expectHitTarget works in the element's own frame coordinates;
            // CDP quads are in root-viewport coordinates.
            let mut local = point;
            if let Some(owner) = &el.frame_owner {
                let model = self
                    .page
                    .send("DOM.getBoxModel", json!({"objectId": owner}))
                    .await?;
                let content = model
                    .pointer("/model/content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                local.x -= content.first().and_then(Value::as_f64).unwrap_or_default();
                local.y -= content.get(1).and_then(Value::as_f64).unwrap_or_default();
            }
            let hit = self
                .page
                .element_call(
                    el,
                    "(i, node, point) => i.expectHitTarget(point, node)",
                    &[json!({"x": local.x, "y": local.y})],
                    true,
                )
                .await?;
            if hit != Value::String("done".into()) {
                let description = hit
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| hit.to_string());
                return Ok(Err(format!("{description} intercepts pointer events")));
            }
        }
        Ok(Ok(point))
    }

    async fn pointer_action(&self, action: Action, o: &ClickOptions, what: &str) -> Result<()> {
        let timeout = o.timeout.unwrap_or_else(|| self.timeout());
        self.retry(what, timeout, |_| async move {
            let el = match self.resolved(timeout).await? {
                Attempt::Done(el) => el,
                Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
            };
            if !o.force
                && let Some(missing) = self.check_states(&el, action).await?
            {
                return Ok(Attempt::Retry(missing));
            }
            let point = match self.pointer_point(&el, o.force, o.position).await? {
                Ok(p) => p,
                Err(reason) => return Ok(Attempt::Retry(reason)),
            };
            if o.trial {
                return Ok(Attempt::Done(()));
            }
            self.annotate(&el, Some(point)).await;
            let keyboard = self.page.keyboard();
            for m in &o.modifiers {
                keyboard.down(m).await?;
            }
            let mouse = self.page.mouse();
            match action {
                Action::Hover => mouse.move_to(point.x, point.y, 1).await?,
                Action::Tap => self.page.touchscreen().tap(point.x, point.y).await?,
                _ => {
                    mouse
                        .click_with(point.x, point.y, o.button, o.click_count)
                        .await?
                }
            }
            for m in o.modifiers.iter().rev() {
                keyboard.up(m).await?;
            }
            Ok(Attempt::Done(()))
        })
        .await
    }

    // ---- actions ------------------------------------------------------------

    /// `locator.click()`; chain options, then `.await`.
    pub fn click(&self) -> Click<'_> {
        Click {
            locator: self,
            action: Action::Click,
            options: ClickOptions::default(),
        }
    }
    /// `locator.dblclick()`.
    pub fn dblclick(&self) -> Click<'_> {
        Click {
            locator: self,
            action: Action::DblClick,
            options: ClickOptions {
                click_count: 2,
                ..Default::default()
            },
        }
    }
    /// `locator.hover()`.
    pub fn hover(&self) -> Click<'_> {
        Click {
            locator: self,
            action: Action::Hover,
            options: ClickOptions::default(),
        }
    }
    /// `locator.tap()` (needs `has_touch`).
    pub fn tap(&self) -> Click<'_> {
        Click {
            locator: self,
            action: Action::Tap,
            options: ClickOptions::default(),
        }
    }

    /// `locator.fill(value)`.
    pub async fn fill(&self, value: &str) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "fill",
                json!({"selector": self.selector, "value": value}),
                self.fill_impl(value),
            )
            .await
    }

    async fn fill_impl(&self, value: &str) -> Result<()> {
        let timeout = self.timeout();
        self.retry("filling", timeout, |_| async move {
            let el = match self.resolved(timeout).await? {
                Attempt::Done(el) => el,
                Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
            };
            if let Some(missing) = self.check_states(&el, Action::Fill).await? {
                return Ok(Attempt::Retry(missing));
            }
            self.annotate(&el, None).await;
            let r = self
                .page
                .element_call(
                    &el,
                    "(i, node, value) => i.fill(node, value)",
                    &[json!(value)],
                    true,
                )
                .await?;
            match r.as_str() {
                Some("done") => {}
                Some("needsinput") => {
                    if value.is_empty() {
                        self.page.keyboard().press("Delete").await?;
                    } else {
                        self.page.keyboard().insert_text(value).await?;
                    }
                }
                Some(other) => return Ok(Attempt::Retry(other.to_owned())),
                None => {}
            }
            Ok(Attempt::Done(()))
        })
        .await
    }

    /// `locator.clear()`.
    pub async fn clear(&self) -> Result<()> {
        self.fill("").await
    }

    async fn focused(&self, what: &str) -> Result<()> {
        let timeout = self.timeout();
        self.retry(what, timeout, |_| async move {
            let el = match self.resolved(timeout).await? {
                Attempt::Done(el) => el,
                Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
            };
            self.annotate(&el, None).await;
            let r = self
                .page
                .element_call(&el, "(i, node) => i.focusNode(node, true)", &[], true)
                .await?;
            Ok(if r == "done" {
                Attempt::Done(())
            } else {
                Attempt::Retry(r.to_string())
            })
        })
        .await
    }

    /// Shows the screencast action annotation (`screencast.showActions()`)
    /// before an input action on `el`.
    async fn annotate(&self, el: &ElementRef, point: Option<Point>) {
        if !self.page.showing_actions() {
            return;
        }
        let bbox = self.page.box_of(el).await.ok().flatten();
        let locator = self.to_playwright_string().await.ok();
        self.page
            .show_action(point.map(|p| (p.x, p.y)), bbox, locator.as_deref())
            .await;
    }

    /// `locator.drop(payload)`: an external drag-and-drop of files and/or
    /// data onto the element's centre (`dragenter`, `dragover`, `drop`).
    pub async fn drop(&self, payload: DropPayload) -> Result<()> {
        if payload.files.is_empty() && payload.paths.is_empty() && payload.data.is_empty() {
            return Err(Error::InvalidArgument(
                "At least one of \"files\" or \"data\" must be provided.".into(),
            ));
        }
        let fs = self.page.inner.shared.services.fs.clone();
        let mut files: Vec<Value> = Vec::new();
        for p in &payload.paths {
            let bytes = fs.read(p)?;
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mime = crate::domain::network::mime_type_for_path(&name)
                .unwrap_or("application/octet-stream");
            files.push(json!({"name": name, "mimeType": mime,
                              "buffer": base64::engine::general_purpose::STANDARD.encode(bytes)}));
        }
        for f in &payload.files {
            files.push(json!({"name": f.name, "mimeType": f.mime_type,
                              "buffer": base64::engine::general_purpose::STANDARD.encode(&f.buffer)}));
        }
        let data: Vec<Value> = payload
            .data
            .iter()
            .map(|(m, v)| json!({"mimeType": m, "value": v}))
            .collect();
        let timeout = self.timeout();
        let (files, data) = (&files, &data);
        self.retry("dropping", timeout, |_| async move {
            let el = match self.resolved(timeout).await? {
                Attempt::Done(el) => el,
                Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
            };
            if let Some(missing) = self.check_states(&el, Action::Hover).await? {
                return Ok(Attempt::Retry(missing));
            }
            let point = match self.pointer_point(&el, false, None).await? {
                Ok(p) => p,
                Err(reason) => return Ok(Attempt::Retry(reason)),
            };
            let r = self
                .page
                .element_call(&el, DROP, &[json!(files), json!(data), json!({"x": point.x, "y": point.y})], true)
                .await?;
            match r.as_str() {
                Some("accepted") => Ok(Attempt::Done(())),
                Some("not-accepted") => Err(Error::Evaluation(
                    "Drop target did not accept the drop \u{2014} its dragover handler did not call preventDefault()".into(),
                )),
                _ => Ok(Attempt::Retry("element is not attached to the DOM".into())),
            }
        })
        .await
    }

    /// `locator.focus()`.
    pub async fn focus(&self) -> Result<()> {
        self.focused("focusing").await
    }

    /// `locator.blur()`.
    pub async fn blur(&self) -> Result<()> {
        let el = self.element("blurring", self.timeout()).await?;
        self.page
            .element_call(&el, "(i, node) => i.blurNode(node)", &[], true)
            .await?;
        Ok(())
    }

    /// `locator.press("Control+A")`.
    pub async fn press(&self, key: &str) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "press",
                json!({"selector": self.selector, "key": key}),
                async {
                    self.focused("pressing").await?;
                    self.page.keyboard().press(key).await
                },
            )
            .await
    }

    /// `locator.pressSequentially(text)` (Playwright's former `type`).
    pub async fn press_sequentially(&self, text: &str) -> Result<()> {
        self.focused("typing").await?;
        self.page.keyboard().type_text(text).await
    }

    /// `locator.setChecked(checked)`.
    pub async fn set_checked(&self, checked: bool) -> Result<()> {
        self.page
            .traced(
                "Frame",
                if checked { "check" } else { "uncheck" },
                json!({"selector": self.selector}),
                self.set_checked_impl(checked),
            )
            .await
    }

    async fn set_checked_impl(&self, checked: bool) -> Result<()> {
        let state = if checked { "checked" } else { "unchecked" };
        let timeout = self.timeout();
        let el = self.element("checking", timeout).await?;
        let current = self
            .page
            .element_call(
                &el,
                "(i, node, s) => i.elementState(node, s)",
                &[json!(state)],
                true,
            )
            .await?;
        if current["matches"] == Value::Bool(true) {
            return Ok(());
        }
        self.pointer_action(Action::Check, &ClickOptions::default(), "checking")
            .await?;
        let el = self.element("checking", timeout).await?;
        let after = self
            .page
            .element_call(
                &el,
                "(i, node, s) => i.elementState(node, s)",
                &[json!(state)],
                true,
            )
            .await?;
        if after["matches"] != Value::Bool(true) {
            return Err(Error::Evaluation(
                "Clicking the checkbox did not change its state".into(),
            ));
        }
        Ok(())
    }

    /// `locator.check()`.
    pub async fn check(&self) -> Result<()> {
        self.set_checked(true).await
    }

    /// `locator.uncheck()`.
    pub async fn uncheck(&self) -> Result<()> {
        self.set_checked(false).await
    }

    /// `locator.selectOption(values)` (each matches an option's value or label);
    /// resolves with the selected values.
    pub fn select_option(&self, values: &[&str]) -> SelectOption<'_> {
        SelectOption {
            locator: self,
            values: values.iter().map(|v| (*v).to_owned()).collect(),
            timeout: None,
        }
    }

    async fn run_select_option(&self, values: &[String], timeout: Duration) -> Result<Vec<String>> {
        self.page
            .traced(
                "Frame",
                "selectOption",
                json!({"selector": self.selector, "options": values}),
                self.run_select_option_impl(values, timeout),
            )
            .await
    }

    async fn run_select_option_impl(
        &self,
        values: &[String],
        timeout: Duration,
    ) -> Result<Vec<String>> {
        let options: Vec<Value> = values.iter().map(|v| json!({"valueOrLabel": v})).collect();
        self.retry("selecting options", timeout, |_| {
            let options = options.clone();
            async move {
                let el = match self.resolved(timeout).await? {
                    Attempt::Done(el) => el,
                    Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
                };
                if let Some(missing) = self.check_states(&el, Action::SelectOption).await? {
                    return Ok(Attempt::Retry(missing));
                }
                let r = self
                    .page
                    .element_call(
                        &el,
                        "(i, node, o) => i.selectOptions(node, o)",
                        &[json!(options)],
                        true,
                    )
                    .await?;
                Ok(match r {
                    Value::Array(a) => Attempt::Done(
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect(),
                    ),
                    Value::String(e) => Attempt::Retry(match e.as_str() {
                        "error:optionsnotfound" => "did not find some options".to_owned(),
                        "error:optionnotenabled" => {
                            "option being selected is not enabled".to_owned()
                        }
                        other => other.to_owned(),
                    }),
                    other => Attempt::Retry(other.to_string()),
                })
            }
        })
        .await
    }

    /// `locator.setInputFiles(paths)`.
    pub async fn set_input_files(&self, files: &[PathBuf]) -> Result<()> {
        let el = self.element("setting input files", self.timeout()).await?;
        let target = self
            .page
            .element_call(
                &el,
                "(i, node) => { const e = i.retarget(node, 'follow-label'); if (!e || e.nodeName !== 'INPUT' || e.type !== 'file') throw i.createStacklessError('Node is not an HTMLInputElement of type file'); return e; }",
                &[],
                false,
            )
            .await?;
        let object_id = target.get("objectId").cloned().unwrap_or(Value::Null);
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
                json!({"files": files, "objectId": object_id}),
            )
            .await?;
        Ok(())
    }

    /// `locator.dispatchEvent(type)`.
    pub async fn dispatch_event(&self, kind: &str) -> Result<()> {
        self.dispatch_event_with(kind, json!({})).await
    }

    /// `locator.dispatchEvent(type, eventInit)`.
    pub async fn dispatch_event_with(&self, kind: &str, init: Value) -> Result<()> {
        let el = self.element("dispatching an event", self.timeout()).await?;
        self.page
            .element_call(
                &el,
                "(i, node, type, init) => i.dispatchEvent(node, type, init)",
                &[json!(kind), init],
                true,
            )
            .await?;
        Ok(())
    }

    /// `locator.scrollIntoViewIfNeeded()`.
    pub async fn scroll_into_view_if_needed(&self) -> Result<()> {
        let timeout = self.timeout();
        self.retry("scrolling into view", timeout, |_| async move {
            let el = match self.resolved(timeout).await? {
                Attempt::Done(el) => el,
                Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
            };
            if let Some(missing) = self.check_states(&el, Action::ScrollIntoView).await? {
                return Ok(Attempt::Retry(missing));
            }
            Ok(
                match self
                    .page
                    .send(
                        "DOM.scrollIntoViewIfNeeded",
                        json!({"objectId": el.object_id}),
                    )
                    .await
                {
                    Ok(_) => Attempt::Done(()),
                    Err(e) => Attempt::Retry(e.to_string()),
                },
            )
        })
        .await
    }

    /// `locator.screenshot()` of just this element.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        self.screenshot_with(ScreenshotOptions::default()).await
    }

    /// `locator.screenshot(options)`.
    pub async fn screenshot_with(&self, options: ScreenshotOptions) -> Result<Vec<u8>> {
        self.scroll_into_view_if_needed().await?;
        let el = self.element("taking a screenshot", self.timeout()).await?;
        self.page
            .run_screenshot_of(&options, self.timeout(), Some(&el))
            .await
    }

    // ---- queries --------------------------------------------------------------

    async fn read(&self, what: &str, body: &str) -> Result<Value> {
        let el = self.element(what, self.timeout()).await?;
        self.page.element_call(&el, body, &[], true).await
    }

    /// `locator.textContent()`.
    pub async fn text_content(&self) -> Result<Option<String>> {
        let v = self
            .read("reading text", "(i, node) => node.textContent")
            .await?;
        Ok(v.as_str().map(str::to_owned))
    }

    /// `locator.innerText()`.
    pub async fn inner_text(&self) -> Result<String> {
        let v = self
            .read(
                "reading inner text",
                "(i, node) => { if (node.namespaceURI !== 'http://www.w3.org/1999/xhtml') throw i.createStacklessError('Node is not an HTMLElement'); return node.innerText; }",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.innerHTML()`.
    pub async fn inner_html(&self) -> Result<String> {
        let v = self
            .read("reading inner HTML", "(i, node) => node.innerHTML")
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.getAttribute(name)`.
    pub async fn get_attribute(&self, name: &str) -> Result<Option<String>> {
        let el = self.element("reading an attribute", self.timeout()).await?;
        let v = self
            .page
            .element_call(
                &el,
                "(i, node, n) => node.getAttribute(n)",
                &[json!(name)],
                true,
            )
            .await?;
        Ok(v.as_str().map(str::to_owned))
    }

    /// `locator.inputValue()`.
    pub async fn input_value(&self) -> Result<String> {
        let v = self
            .read(
                "reading the input value",
                "(i, node) => { const e = i.retarget(node, 'follow-label'); if (!e || !['INPUT', 'TEXTAREA', 'SELECT'].includes(e.nodeName)) throw i.createStacklessError('Node is not an <input>, <textarea> or <select> element'); return e.value; }",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    async fn state(&self, state: &str) -> Result<bool> {
        let el = self
            .element(&format!("checking {state}"), self.timeout())
            .await?;
        let v = self
            .page
            .element_call(
                &el,
                "(i, node, s) => i.elementState(node, s)",
                &[json!(state)],
                true,
            )
            .await?;
        Ok(v["matches"] == Value::Bool(true))
    }

    /// `locator.isVisible()` (does not wait).
    pub async fn is_visible(&self) -> Result<bool> {
        match self.query(true, self.timeout()).await? {
            None => Ok(false),
            Some(el) => {
                let v = self
                    .page
                    .element_call(
                        &el,
                        "(i, node) => i.elementState(node, 'visible')",
                        &[],
                        true,
                    )
                    .await?;
                Ok(v["matches"] == Value::Bool(true))
            }
        }
    }
    /// `locator.isHidden()` (does not wait).
    pub async fn is_hidden(&self) -> Result<bool> {
        Ok(!self.is_visible().await?)
    }
    pub async fn is_enabled(&self) -> Result<bool> {
        self.state("enabled").await
    }
    pub async fn is_disabled(&self) -> Result<bool> {
        self.state("disabled").await
    }
    pub async fn is_editable(&self) -> Result<bool> {
        self.state("editable").await
    }
    pub async fn is_checked(&self) -> Result<bool> {
        self.state("checked").await
    }

    /// `locator.count()` (does not wait).
    pub async fn count(&self) -> Result<usize> {
        let Some((frame, selector, _)) = self.scope(self.timeout()).await? else {
            return Ok(0);
        };
        let v = self
            .page
            .injected_call(
                frame.as_deref(),
                "(i, selector) => i.querySelectorAll(i.parseSelector(selector), document).length",
                &[json!(selector)],
                true,
                self.timeout(),
            )
            .await?;
        Ok(v.as_u64().unwrap_or_default() as usize)
    }

    /// `locator.all()`: one locator per currently matching element.
    pub async fn all(&self) -> Result<Vec<Locator>> {
        let n = self.count().await?;
        Ok((0..n as i64).map(|i| self.nth(i)).collect())
    }

    async fn all_strings(&self, body: &str) -> Result<Vec<String>> {
        let Some((frame, selector, _)) = self.scope(self.timeout()).await? else {
            return Ok(Vec::new());
        };
        let v = self
            .page
            .injected_call(
                frame.as_deref(),
                &format!("(i, selector) => i.querySelectorAll(i.parseSelector(selector), document).map({body})"),
                &[json!(selector)],
                true,
                self.timeout(),
            )
            .await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|s| s.as_str().unwrap_or_default().to_owned())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `locator.allInnerTexts()`.
    pub async fn all_inner_texts(&self) -> Result<Vec<String>> {
        self.all_strings("e => e.innerText").await
    }

    /// `locator.allTextContents()`.
    pub async fn all_text_contents(&self) -> Result<Vec<String>> {
        self.all_strings("e => e.textContent || ''").await
    }

    /// `locator.boundingBox()`.
    pub async fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        let el = self
            .element("reading the bounding box", self.timeout())
            .await?;
        self.page.box_of(&el).await
    }

    /// `locator.evaluate(fn)`: `fn(element)` in the page's main world.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_with(function, Value::Null).await
    }

    /// `locator.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let el = self.element("evaluating", self.timeout()).await?;
        let node = self
            .page
            .send("DOM.describeNode", json!({"objectId": el.object_id}))
            .await?;
        let backend = node
            .pointer("/node/backendNodeId")
            .cloned()
            .unwrap_or(Value::Null);
        let frame = self.scope(self.timeout()).await?.and_then(|(f, _, _)| f);
        let main = self
            .page
            .frame_context(frame.as_deref(), World::Main, self.timeout())
            .await?;
        let adopted = self
            .page
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": backend, "executionContextId": main}),
            )
            .await?;
        let object_id = adopted
            .pointer("/object/objectId")
            .cloned()
            .unwrap_or(Value::Null);
        let source = function.trim().trim_end_matches(';');
        let declaration = format!(
            "async function(arg) {{ const __v = (\n{source}\n); return typeof __v === 'function' ? await __v(this, arg) : __v; }}"
        );
        let r = self
            .page
            .send(
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": declaration, "objectId": object_id,
                       "arguments": [{"value": serde_json::to_value(arg)?}], "returnByValue": true, "awaitPromise": true}),
            )
            .await?;
        let v = crate::application::js::result_value(&r)?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `locator.waitFor({ state })`; chain `.timeout()`, then `.await`.
    pub fn wait_for(&self, state: WaitForState) -> WaitFor<'_> {
        WaitFor {
            locator: self,
            state,
            timeout: None,
        }
    }

    async fn run_wait_for(&self, state: WaitForState, timeout: Duration) -> Result<()> {
        self.retry("waiting for", timeout, |_| async move {
            let el = self.query(true, timeout).await?;
            let visible = match &el {
                Some(el) => {
                    let v = self
                        .page
                        .element_call(
                            el,
                            "(i, node) => i.elementState(node, 'visible')",
                            &[],
                            true,
                        )
                        .await?;
                    v["matches"] == Value::Bool(true)
                }
                None => false,
            };
            let done = match state {
                WaitForState::Attached => el.is_some(),
                WaitForState::Detached => el.is_none(),
                WaitForState::Visible => visible,
                WaitForState::Hidden => !visible,
            };
            Ok(if done {
                Attempt::Done(())
            } else {
                Attempt::Retry(format!("waiting for {state:?}"))
            })
        })
        .await
    }
}

/// Click / hover / tap options.
#[derive(Debug, Clone)]
pub struct ClickOptions {
    pub button: MouseButton,
    pub click_count: u32,
    /// Offset from the element's top-left corner (border box).
    pub position: Option<Point>,
    /// `Alt`, `Control`, `ControlOrMeta`, `Meta`, `Shift` held during the action.
    pub modifiers: Vec<String>,
    /// Skip actionability checks.
    pub force: bool,
    /// Only perform the checks.
    pub trial: bool,
    pub timeout: Option<Duration>,
}

impl Default for ClickOptions {
    fn default() -> Self {
        Self {
            button: MouseButton::Left,
            click_count: 1,
            position: None,
            modifiers: Vec::new(),
            force: false,
            trial: false,
            timeout: None,
        }
    }
}

/// A pending click / dblclick / hover / tap.
#[must_use = "does nothing until awaited"]
pub struct Click<'a> {
    locator: &'a Locator,
    action: Action,
    options: ClickOptions,
}

impl<'a> Click<'a> {
    pub fn button(mut self, button: MouseButton) -> Self {
        self.options.button = button;
        self
    }
    pub fn click_count(mut self, n: u32) -> Self {
        self.options.click_count = n;
        self
    }
    pub fn position(mut self, x: f64, y: f64) -> Self {
        self.options.position = Some(Point { x, y });
        self
    }
    pub fn modifiers(mut self, modifiers: &[&str]) -> Self {
        self.options.modifiers = modifiers.iter().map(|m| (*m).to_owned()).collect();
        self
    }
    pub fn force(mut self, force: bool) -> Self {
        self.options.force = force;
        self
    }
    pub fn trial(mut self, trial: bool) -> Self {
        self.options.trial = trial;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.options.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for Click<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let what = match self.action {
                Action::Hover => "hovering",
                Action::Tap => "tapping",
                Action::DblClick => "double-clicking",
                _ => "clicking",
            };
            let method = match self.action {
                Action::Hover => "hover",
                Action::Tap => "tap",
                Action::DblClick => "dblclick",
                _ => "click",
            };
            self.locator
                .page
                .traced(
                    "Frame",
                    method,
                    json!({"selector": self.locator.selector}),
                    self.locator
                        .pointer_action(self.action, &self.options, what),
                )
                .await
        }
        .boxed()
    }
}

/// A pending `locator.selectOption()`.
#[must_use = "does nothing until awaited"]
pub struct SelectOption<'a> {
    locator: &'a Locator,
    values: Vec<String>,
    timeout: Option<Duration>,
}

impl<'a> SelectOption<'a> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for SelectOption<'a> {
    type Output = Result<Vec<String>>;
    type IntoFuture = BoxFuture<'a, Self::Output>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.locator.timeout());
            self.locator.run_select_option(&self.values, timeout).await
        }
        .boxed()
    }
}

/// A pending `locator.waitFor()`.
#[must_use = "does nothing until awaited"]
pub struct WaitFor<'a> {
    locator: &'a Locator,
    state: WaitForState,
    timeout: Option<Duration>,
}

impl<'a> WaitFor<'a> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for WaitFor<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.locator.timeout());
            self.locator.run_wait_for(self.state, timeout).await
        }
        .boxed()
    }
}

/// Splits a selector at `internal:control=enter-frame` hops.
fn split_frames(selector: &str) -> Vec<String> {
    static HOP: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let hop = HOP.get_or_init(|| {
        regex::Regex::new(r"\s*>>\s*internal:control=enter-frame\s*(?:>>\s*)?").expect("valid")
    });
    hop.split(selector).map(str::to_owned).collect()
}

/// Locates inside an iframe (Playwright's `FrameLocator`).
#[derive(Clone)]
pub struct FrameLocator {
    page: Page,
    frame: Option<String>,
    /// Selector of the `<iframe>` element.
    selector: String,
}

impl FrameLocator {
    pub(crate) fn page_of(&self) -> &Page {
        &self.page
    }
}

impl std::fmt::Debug for FrameLocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frameLocator({:?})", self.selector)
    }
}

impl FrameLocator {
    fn inside(&self, selector: &str) -> String {
        compose::chain(&compose::enter_frame(&self.selector), selector)
    }
    pub fn locator(&self, selector: &str) -> Locator {
        Locator::in_frame(&self.page, self.frame.clone(), self.inside(selector))
    }
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            frame: self.frame.clone(),
            selector: self.inside(selector),
        }
    }
    pub fn get_by_role(&self, role: &str, options: RoleOptions) -> Locator {
        self.locator(&selectors::get_by_role(role, &options))
    }
    pub fn get_by_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_text(&text.into(), exact))
    }
    pub fn get_by_label(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_label(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id(
            &self.page_ref().test_id_attribute(),
            &id.into(),
        ))
    }
    /// The `<iframe>` element itself.
    pub fn owner(&self) -> Locator {
        Locator::in_frame(&self.page, self.frame.clone(), self.selector.clone())
    }
    pub fn first(&self) -> FrameLocator {
        self.nth(0)
    }
    pub fn last(&self) -> FrameLocator {
        self.nth(-1)
    }
    pub fn nth(&self, index: i64) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            frame: self.frame.clone(),
            selector: compose::nth(&self.selector, index),
        }
    }
}

impl Page {
    /// `page.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.clone(),
            frame: None,
            selector: selector.to_owned(),
        }
    }
}

// ---- extras: handles, evaluate_all, aria, descriptions --------------------

/// Selector engine resolving element handles registered by id (utility world).
pub(crate) const HANDLE_ENGINE: &str = "({ queryAll(root, id) { const m = globalThis.__chromiumoxide_handles; const e = m && m.get(id); return e && e.isConnected && (root === e || root.contains(e)) ? [e] : []; } })";

impl Locator {
    /// Adopts an element into the main world of its frame; the object id.
    async fn main_world_object(&self, el: &ElementRef) -> Result<Value> {
        let node = self
            .page
            .send("DOM.describeNode", json!({"objectId": el.object_id}))
            .await?;
        let backend = node
            .pointer("/node/backendNodeId")
            .cloned()
            .unwrap_or(Value::Null);
        let frame = self.scope(self.timeout()).await?.and_then(|(f, _, _)| f);
        let main = self
            .page
            .frame_context(frame.as_deref(), World::Main, self.timeout())
            .await?;
        let adopted = self
            .page
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": backend, "executionContextId": main}),
            )
            .await?;
        Ok(adopted["object"].clone())
    }

    /// Every matching element, as main-world remote objects.
    async fn all_main_world_objects(&self) -> Result<Vec<Value>> {
        let Some((frame, selector, frame_owner)) = self.scope(self.timeout()).await? else {
            return Ok(Vec::new());
        };
        let list = self
            .page
            .injected_call(
                frame.as_deref(),
                "(i, selector) => i.querySelectorAll(i.parseSelector(selector), document)",
                &[json!(selector)],
                false,
                self.timeout(),
            )
            .await?;
        let Some(list_id) = list.get("objectId").and_then(Value::as_str) else {
            return Ok(Vec::new());
        };
        let props = self
            .page
            .send(
                "Runtime.getProperties",
                json!({"objectId": list_id, "ownProperties": true}),
            )
            .await?;
        let mut out = Vec::new();
        for p in props
            .get("result")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let is_index = p
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|n| n.parse::<usize>().is_ok());
            if let (true, Some(id)) = (
                is_index,
                p.pointer("/value/objectId").and_then(Value::as_str),
            ) {
                let el = ElementRef {
                    object_id: id.to_owned(),
                    frame_owner: frame_owner.clone(),
                };
                out.push(self.main_world_object(&el).await?);
            }
        }
        Ok(out)
    }

    /// The frame the matched elements live in (after iframe hops).
    async fn frame_for_handles(&self) -> Result<Option<String>> {
        Ok(self.scope(self.timeout()).await?.and_then(|(f, _, _)| f))
    }

    /// `locator.evaluateHandle(fn)`: `fn(element)`, returned as a handle.
    pub async fn evaluate_handle(
        &self,
        function: &str,
    ) -> Result<crate::application::handle::JsHandle> {
        let el = self.element("evaluating", self.timeout()).await?;
        let handle = crate::application::handle::JsHandle::from_remote(
            &self.page,
            &self.main_world_object(&el).await?,
        );
        handle.evaluate_handle(function).await
    }

    /// `locator.elementHandle()`: the (single) element.
    pub async fn element_handle(&self) -> Result<crate::application::handle::JsHandle> {
        let el = self.element("getting the element", self.timeout()).await?;
        let mut h = crate::application::handle::JsHandle::from_remote(
            &self.page,
            &self.main_world_object(&el).await?,
        );
        h.frame = self.frame_for_handles().await?;
        Ok(h)
    }

    /// `locator.elementHandles()`: every matching element.
    pub async fn element_handles(&self) -> Result<Vec<crate::application::handle::JsHandle>> {
        let frame = self.frame_for_handles().await?;
        Ok(self
            .all_main_world_objects()
            .await?
            .iter()
            .map(|o| {
                let mut h = crate::application::handle::JsHandle::from_remote(&self.page, o);
                h.frame = frame.clone();
                h
            })
            .collect())
    }

    /// `locator.evaluateAll(fn)`: `fn(elements)` in the page.
    pub async fn evaluate_all<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_all_with(function, Value::Null).await
    }

    /// `locator.evaluateAll(fn, arg)`.
    pub async fn evaluate_all_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let objects = self.all_main_world_objects().await?;
        let source = function.trim().trim_end_matches(';');
        let declaration = format!(
            "async function(arg, ...els) {{ const __v = (\n{source}\n); return typeof __v === 'function' ? await __v(els, arg) : __v; }}"
        );
        let mut arguments = vec![json!({"value": serde_json::to_value(arg)?})];
        arguments.extend(objects.iter().map(|o| json!({"objectId": o["objectId"]})));
        let mut params = json!({"functionDeclaration": declaration, "arguments": arguments,
                                "returnByValue": true, "awaitPromise": true});
        match objects.first() {
            Some(o) => params["objectId"] = o["objectId"].clone(),
            None => {
                let frame = self.scope(self.timeout()).await?.and_then(|(f, _, _)| f);
                params["executionContextId"] = json!(
                    self.page
                        .frame_context(frame.as_deref(), World::Main, self.timeout())
                        .await?
                );
            }
        }
        let r = self.page.send("Runtime.callFunctionOn", params).await?;
        let v = crate::application::js::result_value(&r)?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `locator.ariaSnapshot()`: the accessibility tree as YAML.
    pub async fn aria_snapshot(&self) -> Result<String> {
        let el = self
            .element("taking an aria snapshot", self.timeout())
            .await?;
        let v = self
            .page
            .element_call(
                &el,
                "(i, node) => i.ariaSnapshot(node, {mode: 'default'})",
                &[],
                true,
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.ariaSnapshotJSON()`: the accessibility tree as JSON nodes.
    pub async fn aria_snapshot_json(&self) -> Result<Value> {
        let el = self
            .element("taking an aria snapshot", self.timeout())
            .await?;
        self.page
            .element_call(
                &el,
                "(i, node) => i.ariaSnapshotJSON(node, {mode: 'default'}).json",
                &[],
                true,
            )
            .await
    }

    /// Playwright's `locator.toString()` (`getByRole('button', { name: 'Go' })`),
    /// computed by Playwright's own generator in the page.
    pub async fn to_playwright_string(&self) -> Result<String> {
        if let Some(d) = self.description() {
            return Ok(d);
        }
        let frame = self.frame.clone();
        let v = self
            .page
            .injected_call(
                frame.as_deref(),
                "(i, selector) => i.utils.asLocator('javascript', selector)",
                &[json!(self.selector)],
                true,
                self.timeout(),
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.describe(description)`: a description for reports and traces.
    pub fn describe(&self, description: &str) -> Locator {
        self.with(format!(
            "{} >> internal:describe={}",
            self.selector,
            serde_json::to_string(description).unwrap_or_default()
        ))
    }

    /// `locator.description()`: the description given with `describe`.
    pub fn description(&self) -> Option<String> {
        let (_, last) = self.selector.rsplit_once(" >> ")?;
        let value = last.strip_prefix("internal:describe=")?;
        serde_json::from_str(value).ok()
    }

    /// `locator.normalize()`: the same element through Playwright's preferred locator.
    pub async fn normalize(&self) -> Result<Locator> {
        let el = self.element("normalizing", self.timeout()).await?;
        let v = self
            .page
            .element_call(
                &el,
                "(i, node) => i.generateSelectorSimple(node)",
                &[],
                true,
            )
            .await?;
        let selector = v
            .as_str()
            .ok_or_else(|| Error::Evaluation("no selector generated".into()))?;
        Ok(Locator::in_frame(
            &self.page,
            self.frame.clone(),
            selector.to_owned(),
        ))
    }

    /// `locator.filter({visible})`.
    pub fn visible(&self, visible: bool) -> Locator {
        self.with(format!("{} >> visible={visible}", self.selector))
    }

    /// `locator.getByAltText(text)` inside this locator.
    pub fn get_by_alt_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_alt_text(&text.into(), exact))
    }

    /// `locator.getByTitle(text)` inside this locator.
    pub fn get_by_title(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_title(&text.into(), exact))
    }

    /// `locator.selectText()`.
    pub async fn select_text(&self) -> Result<()> {
        let el = self.element("selecting text", self.timeout()).await?;
        self.page
            .element_call(&el, "(i, node) => i.selectText(node)", &[], true)
            .await?;
        Ok(())
    }

    /// `locator.type(text)` (deprecated in Playwright: use `press_sequentially`).
    #[deprecated(note = "use press_sequentially, like Playwright")]
    pub async fn type_text(&self, text: &str) -> Result<()> {
        self.press_sequentially(text).await
    }

    /// `locator.waitForFunction(fn)`: waits until `fn(element)` is truthy.
    pub async fn wait_for_function(&self, function: &str) -> Result<()> {
        let timeout = self.timeout();
        let source = function.to_owned();
        self.retry("waiting for function", timeout, |_| {
            let source = source.clone();
            async move {
                if self.count().await? == 0 {
                    return Ok(Attempt::Retry(format!("waiting for {self:?}")));
                }
                let v: Value = self.evaluate(&source).await?;
                let truthy = !matches!(v, Value::Null | Value::Bool(false))
                    && v != json!(0)
                    && v != json!("");
                Ok(if truthy {
                    Attempt::Done(())
                } else {
                    Attempt::Retry("function returned a falsy value".into())
                })
            }
        })
        .await
    }

    /// `locator.highlight()`: outlines the matching elements.
    pub async fn highlight(&self) -> Result<()> {
        let Some((frame, selector, _)) = self.scope(self.timeout()).await? else {
            return Ok(());
        };
        self.page
            .injected_call(
                frame.as_deref(),
                "(i, selector) => i.setHighlights([{ selector: i.parseSelector(selector) }])",
                &[json!(selector)],
                true,
                self.timeout(),
            )
            .await?;
        Ok(())
    }

    /// `locator.hideHighlight()`.
    pub async fn hide_highlight(&self) -> Result<()> {
        self.page.hide_highlight().await
    }
}

impl Page {
    /// The selector of a child frame's `<iframe>` within its parent frame.
    pub(crate) async fn frame_owner_selector(
        &self,
        frame_id: &str,
        parent: Option<&str>,
    ) -> Result<String> {
        let timeout = self.default_timeout();
        let context = self.injected_context(parent, timeout).await?;
        let owner = self
            .send("DOM.getFrameOwner", json!({"frameId": frame_id}))
            .await?;
        let r = self
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": owner.get("backendNodeId"), "executionContextId": context}),
            )
            .await?;
        let el = ElementRef {
            object_id: r
                .pointer("/object/objectId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            frame_owner: None,
        };
        let v = self
            .element_call(
                &el,
                "(i, node) => i.generateSelectorSimple(node)",
                &[],
                true,
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// The `<iframe>` element of a child frame, as a remote object id.
    pub(crate) async fn frame_owner_object(&self, frame_id: &str) -> Result<Option<String>> {
        let owner = self
            .send("DOM.getFrameOwner", json!({"frameId": frame_id}))
            .await?;
        let Some(backend) = owner.get("backendNodeId").cloned() else {
            return Ok(None);
        };
        let r = self
            .send("DOM.resolveNode", json!({"backendNodeId": backend}))
            .await?;
        Ok(r.pointer("/object/objectId")
            .and_then(Value::as_str)
            .map(str::to_owned))
    }

    /// `page.ariaSnapshot()`: the accessibility tree of the page as YAML.
    pub async fn aria_snapshot(&self) -> Result<String> {
        self.locator("body").aria_snapshot().await
    }

    /// `page.ariaSnapshotJSON()`.
    pub async fn aria_snapshot_json(&self) -> Result<Value> {
        self.locator("body").aria_snapshot_json().await
    }

    /// `page.hideHighlight()`.
    pub async fn hide_highlight(&self) -> Result<()> {
        self.injected_call(
            None,
            "(i) => i.setHighlights([])",
            &[],
            true,
            self.default_timeout(),
        )
        .await?;
        Ok(())
    }
}

impl Locator {
    /// Waits until the element has all `states` (`visible`, `stable`, ...).
    pub(crate) async fn wait_for_states(&self, states: &[&str]) -> Result<()> {
        let timeout = self.timeout();
        let states: Vec<String> = states.iter().map(|s| (*s).to_owned()).collect();
        self.retry("waiting for element state", timeout, |_| {
            let states = states.clone();
            async move {
                let el = match self.resolved(timeout).await? {
                    Attempt::Done(el) => el,
                    Attempt::Retry(r) => return Ok(Attempt::Retry(r)),
                };
                let r = self
                    .page
                    .element_call(
                        &el,
                        "async (i, node, states) => await i.checkElementStates(node, states)",
                        &[json!(states)],
                        true,
                    )
                    .await?;
                Ok(match r {
                    Value::Null => Attempt::Done(()),
                    other => Attempt::Retry(format!("element state not reached: {other}")),
                })
            }
        })
        .await
    }
}
