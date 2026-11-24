//! The rest of Playwright's `Locator` for WebKit: element states, blur,
//! clear, dispatchEvent, evaluateAll, selectText, waits, select and file
//! inputs, drag and drop, scrolling, highlights, describe, normalize and
//! external drops. Like the rest of the WebKit locator, each runs
//! Playwright's injected script in the locator's frame.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::locator::{DROP, DropPayload, WaitForState};
use crate::application::webkit::Page;
use crate::application::webkit_frame::FrameLocator;
use crate::application::webkit_locator::Locator;
use crate::domain::error::{Error, Result};
use crate::domain::selectors::{self, TextMatch, compose};

impl std::fmt::Display for Locator {
    /// Playwright's `locator.toString()`: the selector.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for frame in &self.frames {
            write!(f, "{frame} >> internal:control=enter-frame >> ")?;
        }
        f.write_str(&self.selector)
    }
}

impl Locator {
    pub(crate) fn with(&self, selector: String) -> Locator {
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames: self.frames.clone(),
            selector,
        }
    }

    async fn state(&self, state: &str) -> Result<bool> {
        let v = self
            .read(
                "(i, node, state) => { const s = i.elementState(node, state, true); \
                 if (typeof s.received === 'string' && s.received.startsWith('error:')) \
                 throw i.createStacklessError(s.received === 'error:notcheckbox' \
                   ? 'Not a checkbox or radio button' : 'Element is not attached to the DOM'); \
                 return s.matches; }",
                &[json!(state)],
            )
            .await?;
        Ok(v == Value::Bool(true))
    }

    /// `locator.isEnabled()`.
    pub async fn is_enabled(&self) -> Result<bool> {
        self.state("enabled").await
    }

    /// `locator.isDisabled()`.
    pub async fn is_disabled(&self) -> Result<bool> {
        self.state("disabled").await
    }

    /// `locator.isEditable()`.
    pub async fn is_editable(&self) -> Result<bool> {
        self.state("editable").await
    }

    /// `locator.blur()`.
    pub async fn blur(&self) -> Result<()> {
        self.read("(i, node) => { i.blurNode(node); }", &[])
            .await
            .map(drop)
    }

    /// `locator.clear()`: fills an empty value.
    pub async fn clear(&self) -> Result<()> {
        self.fill("").await
    }

    /// `locator.selectText()`.
    pub async fn select_text(&self) -> Result<()> {
        self.read("(i, node) => { i.selectText(node); }", &[])
            .await
            .map(drop)
    }

    /// `locator.dispatchEvent(type)`.
    pub async fn dispatch_event(&self, kind: &str) -> Result<()> {
        self.dispatch_event_with(kind, json!({})).await
    }

    /// `locator.dispatchEvent(type, eventInit)`.
    pub async fn dispatch_event_with(&self, kind: &str, init: Value) -> Result<()> {
        self.read(
            "(i, node, type, init) => { i.dispatchEvent(node, type, init); }",
            &[json!(kind), init],
        )
        .await
        .map(drop)
    }

    /// `locator.evaluateAll(fn)`: `fn(elements)` over every match (does not
    /// wait).
    pub async fn evaluate_all<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_all_with(function, Value::Null).await
    }

    /// `locator.evaluateAll(fn, arg)`.
    pub async fn evaluate_all_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self
            .call(
                &format!(
                    "async (i, selector, arg) => {{ \
                     const els = i.querySelectorAll(i.parseSelector(selector), document); \
                     const v = await ({function})(els, arg); return v === undefined ? null : v; }}"
                ),
                &[json!(self.selector), arg],
            )
            .await?;
        if v.get("__frame_missing").is_some() {
            return Err(Error::Evaluation(format!(
                "frame of locator('{}') is not attached",
                self.selector
            )));
        }
        Ok(serde_json::from_value(v)?)
    }

    /// Like [`read`](Self::read), awaiting `body`'s result (which may be a
    /// promise).
    pub(crate) async fn read_value(&self, body: &str, args: &[Value]) -> Result<Value> {
        self.read(
            &format!(
                "(i, node, ...args) => Promise.resolve(({body})(i, node, ...args)).then(v => v === undefined ? null : v)"
            ),
            args,
        )
        .await
    }

    /// `locator.waitForFunction(fn)`: until `fn(element)` is truthy.
    pub async fn wait_for_function(&self, function: &str) -> Result<()> {
        let deadline = self.page.now() + self.page.timeout();
        loop {
            let v: Value = self
                .read_value(&format!("(i, node) => ({function})(node)"), &[])
                .await?;
            if !matches!(v, Value::Null | Value::Bool(false)) && v != json!(0) && v != json!("") {
                return Ok(());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page.timeout().as_millis() as u64,
                    action: format!("waiting for function on locator('{}')", self.selector),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `locator.waitFor({ state })`: waits until the element is attached,
    /// visible (default), detached or hidden (strict).
    pub async fn wait_for(&self, state: WaitForState) -> Result<()> {
        let wanted = match state {
            WaitForState::Attached => "attached",
            WaitForState::Detached => "detached",
            WaitForState::Visible => "visible",
            WaitForState::Hidden => "hidden",
        };
        let deadline = self.page.now() + self.page.timeout();
        loop {
            let v = self
                .call(
                    "(i, selector, wanted) => { const parsed = i.parseSelector(selector); \
                     const els = i.querySelectorAll(parsed, document); \
                     if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                     const el = els[0]; \
                     const visible = !!el && i.elementState(el, 'visible', true).matches; \
                     if (wanted === 'attached') return !!el; \
                     if (wanted === 'detached') return !el; \
                     if (wanted === 'visible') return visible; \
                     return !visible; }",
                    &[json!(self.selector), json!(wanted)],
                )
                .await?;
            // A missing frame has no element: detached and hidden hold.
            let missing = v.get("__frame_missing").is_some();
            if v == Value::Bool(true) || (missing && matches!(wanted, "detached" | "hidden")) {
                return Ok(());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page.timeout().as_millis() as u64,
                    action: format!("waiting for locator('{}') to be {wanted}", self.selector),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `locator.selectOption(values)`: each value matches an option's value
    /// or label; resolves with the selected values.
    pub async fn select_option(&self, values: &[&str]) -> Result<Vec<String>> {
        let options: Vec<Value> = values.iter().map(|v| json!({"valueOrLabel": v})).collect();
        let deadline = self.page.now() + self.page.timeout();
        loop {
            let r = self
                .read(
                    "(i, node, options) => i.selectOptions(node, options)",
                    &[json!(options)],
                )
                .await?;
            if let Some(values) = r.as_array() {
                return Ok(values
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .collect());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page.timeout().as_millis() as u64,
                    action: format!(
                        "selecting options in locator('{}'): {}",
                        self.selector,
                        r.as_str().unwrap_or_default()
                    ),
                });
            }
            self.page.sleep(Duration::from_millis(50)).await;
        }
    }

    /// The remote object id of the element (waits for it, strict), in the
    /// locator's frame's utility world.
    pub(crate) async fn element_object(&self) -> Result<(Option<String>, String)> {
        self.read(
            "(i, node) => { globalThis.__chromiumoxide_element = node; }",
            &[],
        )
        .await?;
        let Some((frame, _)) = self.scope().await? else {
            return Err(Error::Evaluation("the frame went away".into()));
        };
        let element = self
            .page
            .run_in(
                "(() => { const el = globalThis.__chromiumoxide_element; \
                 delete globalThis.__chromiumoxide_element; return el; })()",
                frame.as_deref(),
                true,
            )
            .await?;
        let object = element["objectId"].as_str().ok_or_else(|| {
            Error::Evaluation(format!("no element for locator('{}')", self.selector))
        })?;
        Ok((frame, object.to_owned()))
    }

    /// `locator.setInputFiles(files)` (`DOM.setInputFiles`).
    pub async fn set_input_files(&self, files: &[std::path::PathBuf]) -> Result<()> {
        let (_, object) = self.element_object().await?;
        let paths: Vec<String> = files
            .iter()
            .map(|f| {
                std::path::absolute(f)
                    .unwrap_or_else(|_| f.clone())
                    .display()
                    .to_string()
            })
            .collect();
        self.page
            .target_call(
                "DOM.setInputFiles",
                json!({"objectId": object, "paths": paths}),
            )
            .await
            .map(drop)
    }

    /// `locator.dragTo(target)`: press on this element's centre, move to the
    /// target's centre in steps (so drag and drop starts) and release.
    pub async fn drag_to(&self, target: &Locator) -> Result<()> {
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        let (tx, ty) = target.actionable_point(&["visible", "stable"]).await?;
        self.page.drag(x, y, tx, ty).await
    }

    /// `locator.scrollIntoViewIfNeeded()`: waits until visible, then
    /// scrolls it into the middle of the viewport when it is not in view.
    pub async fn scroll_into_view_if_needed(&self) -> Result<()> {
        self.wait_for(WaitForState::Visible).await?;
        self.read(
            "(i, node) => { const r = node.getBoundingClientRect(); \
             if (r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth) \
               node.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); }",
            &[],
        )
        .await
        .map(drop)
    }

    /// `locator.tap()`: a touch tap on the element's centre; like
    /// Playwright, only in a context with `hasTouch`.
    pub async fn tap(&self) -> Result<()> {
        if !self.page.inner.options.has_touch {
            return Err(Error::InvalidArgument(
                "The page does not support tap. Use hasTouch context option to enable touch support."
                    .into(),
            ));
        }
        let (x, y) = self
            .actionable_point(&["visible", "enabled", "stable"])
            .await?;
        self.page.tap_at(x, y).await
    }

    /// `locator.highlight()`: outlines the matching elements.
    pub async fn highlight(&self) -> Result<()> {
        self.call(
            "(i, selector) => { i.setHighlights([{ selector: i.parseSelector(selector) }]); }",
            &[json!(self.selector)],
        )
        .await
        .map(drop)
    }

    /// `locator.hideHighlight()`.
    pub async fn hide_highlight(&self) -> Result<()> {
        self.page.hide_highlight().await
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
        serde_json::from_str(last.strip_prefix("internal:describe=")?).ok()
    }

    /// `locator.normalize()`: the same element through Playwright's
    /// preferred selector.
    pub async fn normalize(&self) -> Result<Locator> {
        let v = self
            .read("(i, node) => i.generateSelectorSimple(node)", &[])
            .await?;
        let selector = v
            .as_str()
            .ok_or_else(|| Error::Evaluation("no selector generated".into()))?;
        Ok(self.with(selector.to_owned()))
    }

    /// `locator.filter({ visible })`.
    pub fn visible(&self, visible: bool) -> Locator {
        self.with(format!("{} >> visible={visible}", self.selector))
    }

    /// `locator.within(parent)`: this locator inside each element of `parent`.
    pub fn within(&self, parent: &Locator) -> Locator {
        parent.with(compose::chain_locator(&parent.selector, &self.selector))
    }

    /// `locator.contentFrame()`: the iframe this locator points to.
    pub fn content_frame(&self) -> FrameLocator {
        let mut frames = self.frames.clone();
        frames.push(self.selector.clone());
        FrameLocator {
            page: self.page.clone(),
            root: self.root.clone(),
            frames,
        }
    }

    /// `locator.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        self.locator(selector).content_frame()
    }

    pub fn get_by_alt_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_alt_text(&text.into(), exact))
    }
    pub fn get_by_title(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_title(&text.into(), exact))
    }
    pub fn get_by_label(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_label(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }

    /// `locator.type(text)` (deprecated in Playwright: use `press_sequentially`).
    #[deprecated(note = "use press_sequentially, like Playwright")]
    pub async fn type_text(&self, text: &str) -> Result<()> {
        self.press_sequentially(text).await
    }

    /// `locator.drop(payload)`: an external drag and drop of files and/or
    /// data onto the element's centre (`dragenter`, `dragover`, `drop`).
    pub async fn drop(&self, payload: DropPayload) -> Result<()> {
        if payload.is_empty() {
            return Err(Error::InvalidArgument(
                "At least one of \"files\" or \"data\" must be provided.".into(),
            ));
        }
        let (files, data) = payload.to_json(&*self.page.inner.services.fs)?;
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        let (ox, oy) = self.scope().await?.map(|(_, o)| o).unwrap_or_default();
        let r = self
            .read(
                DROP,
                &[json!(files), json!(data), json!({"x": x - ox, "y": y - oy})],
            )
            .await?;
        match r.as_str() {
            Some("accepted") => Ok(()),
            Some("not-accepted") => Err(Error::Evaluation(
                "Drop target did not accept the drop \u{2014} its dragover handler did not call preventDefault()".into(),
            )),
            _ => Err(Error::Evaluation("element is not attached to the DOM".into())),
        }
    }
}

impl FrameLocator {
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
}

impl Page {
    /// `page.hideHighlight()`.
    pub async fn hide_highlight(&self) -> Result<()> {
        self.injected_call_in(None, "(i) => { i.setHighlights([]); }", &[])
            .await
            .map(drop)
    }
}
