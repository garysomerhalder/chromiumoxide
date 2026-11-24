//! The rest of Playwright's `Locator` for Firefox: element states, blur,
//! clear, dispatchEvent, evaluateAll, selectText, scrolling, highlights,
//! describe, normalize and external drops. Like the rest of the Firefox
//! locator, each runs Playwright's injected script in the locator's frame.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::firefox::Page;
use crate::application::firefox_locator::{FrameLocator, Locator};
use crate::application::locator::{DROP, DropPayload};
use crate::domain::error::{Error, Result};
use crate::domain::selectors::{self, TextMatch, compose};

/// Playwright's default action timeout.
const TIMEOUT: Duration = Duration::from_secs(30);

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

    /// `locator.waitForFunction(fn)`: until `fn(element)` is truthy.
    pub async fn wait_for_function(&self, function: &str) -> Result<()> {
        let deadline = self.page.now() + TIMEOUT;
        loop {
            let v: Value = self
                .read_value(&format!("(i, node) => ({function})(node)"), &[])
                .await?;
            if !matches!(v, Value::Null | Value::Bool(false)) && v != json!(0) && v != json!("") {
                return Ok(());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for function on locator('{}')", self.selector),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `locator.scrollIntoViewIfNeeded()`: waits until visible, then
    /// scrolls it into the middle of the viewport when it is not in view.
    pub async fn scroll_into_view_if_needed(&self) -> Result<()> {
        self.wait_for(crate::application::locator::WaitForState::Visible)
            .await?;
        self.read(
            "(i, node) => { const r = node.getBoundingClientRect(); \
             if (r.top < 0 || r.left < 0 || r.bottom > innerHeight || r.right > innerWidth) \
               node.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); }",
            &[],
        )
        .await
        .map(drop)
    }

    /// `locator.tap()`: Playwright taps only in a context with `hasTouch`,
    /// which WebDriver BiDi cannot set in Firefox.
    pub async fn tap(&self) -> Result<()> {
        Err(Error::InvalidArgument(
            "The page does not support tap. Use hasTouch context option to enable touch support \
             (WebDriver BiDi cannot set hasTouch in Firefox)."
                .into(),
        ))
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
        let (files, data) = payload.to_json(&*self.page.services().fs)?;
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        let (ox, oy) = self.frame_offset().await?;
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

impl Page {
    /// `page.hideHighlight()`.
    pub async fn hide_highlight(&self) -> Result<()> {
        self.injected_call_in(&self.main_context(), "(i) => { i.setHighlights([]); }", &[])
            .await
            .map(drop)
    }
}
