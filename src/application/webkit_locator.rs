//! Locators on the WebKit engine: Playwright's injected script in the
//! utility world, WebKit's `Input.*` commands for the pointer and keyboard.

use std::time::Duration;

use serde_json::{Value, json};

use crate::application::webkit::Page;
use crate::domain::error::{Error, Result};
use crate::domain::selectors::{self, RoleOptions, TextMatch, compose};

/// `page.locator(selector)` and the `get_by_*` builders on a WebKit page.
impl Page {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.clone(),
            root: None,
            selector: selector.to_owned(),
            frames: Vec::new(),
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
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }
}

/// A WebKit locator: a selector resolved lazily, strict like Playwright's.
#[derive(Clone, Debug)]
pub struct Locator {
    pub(crate) page: Page,
    /// The frame the lookup starts in (`None`: the main frame).
    pub(crate) root: Option<String>,
    pub(crate) selector: String,
    /// `frame_locator` selectors from the page down to the frame.
    pub(crate) frames: Vec<String>,
}

impl Locator {
    /// `locator.locator(selector)`: within this locator's element.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
            selector: compose::chain(&self.selector, selector),
            frames: self.frames.clone(),
        }
    }
    /// `locator.nth(index)` (negative counts from the end).
    pub fn nth(&self, index: i64) -> Locator {
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
            selector: compose::nth(&self.selector, index),
            frames: self.frames.clone(),
        }
    }
    pub fn first(&self) -> Locator {
        self.nth(0)
    }
    pub fn last(&self) -> Locator {
        self.nth(-1)
    }
    pub fn get_by_role(&self, role: &str, options: RoleOptions) -> Locator {
        self.locator(&selectors::get_by_role(role, &options))
    }
    pub fn get_by_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_text(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }

    /// The frame this locator runs in (`None`: the main frame) and its
    /// content's offset in the viewport; `None` while a frame is missing.
    pub(crate) async fn scope(&self) -> Result<Option<(Option<String>, (f64, f64))>> {
        let (mut frame, mut offset) = match &self.root {
            None => (None, (0.0, 0.0)),
            Some(root) => match self.page.frame_offset(root).await? {
                Some(offset) => (Some(root.clone()), offset),
                None => return Ok(None),
            },
        };
        for selector in &self.frames {
            match self.page.child_frame(frame.as_deref(), selector).await? {
                Some((child, (x, y))) => {
                    frame = Some(child);
                    offset = (offset.0 + x, offset.1 + y);
                }
                None => return Ok(None),
            }
        }
        Ok(Some((frame, offset)))
    }

    /// `body(injected, ...args)` in the locator's frame; while the frame is
    /// missing, `{ missing: true }` (callers retry or treat it as no
    /// element).
    pub(crate) async fn call(&self, body: &str, args: &[Value]) -> Result<Value> {
        match self.scope().await? {
            Some((frame, _)) => {
                self.page
                    .injected_call_in(frame.as_deref(), body, args)
                    .await
            }
            None => Ok(json!({"missing": true, "__frame_missing": true})),
        }
    }

    /// The page this locator belongs to.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The selector string (Playwright's internal form).
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Runs `body(injected, element, ...args)` on the single element the
    /// locator resolves to, waiting until there is one (strict).
    pub(crate) async fn read(&self, body: &str, args: &[Value]) -> Result<Value> {
        let function = format!(
            "async (i, selector, args) => {{ const parsed = i.parseSelector(selector); \
             const els = i.querySelectorAll(parsed, document); \
             if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
             if (!els.length) return {{ found: false }}; \
             const value = await ({body})(i, els[0], ...args); \
             return {{ found: true, value: value === undefined ? null : value }}; }}"
        );
        let deadline = self.page.now() + self.page.timeout();
        let mut delay = Duration::from_millis(20);
        loop {
            let r = self
                .call(&function, &[json!(self.selector), json!(args)])
                .await?;
            if r["found"] == Value::Bool(true) {
                return Ok(r["value"].clone());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page.timeout().as_millis() as u64,
                    action: format!("waiting for locator('{}')", self.selector),
                });
            }
            self.page.sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(100));
        }
    }

    /// `body(injected, elements)` over every match, right away.
    async fn all_eval(&self, body: &str) -> Result<Value> {
        let function = format!(
            "(i, selector) => {{ const els = i.querySelectorAll(i.parseSelector(selector), document); \
             return ({body})(i, els); }}"
        );
        self.call(&function, &[json!(self.selector)]).await
    }

    /// `locator.count()`.
    pub async fn count(&self) -> Result<usize> {
        Ok(self
            .all_eval("(i, els) => els.length")
            .await?
            .as_u64()
            .unwrap_or(0) as usize)
    }

    /// `locator.allTextContents()`.
    pub async fn all_text_contents(&self) -> Result<Vec<String>> {
        let v = self
            .all_eval("(i, els) => els.map(e => e.textContent || '')")
            .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// `locator.allInnerTexts()`.
    pub async fn all_inner_texts(&self) -> Result<Vec<String>> {
        let v = self
            .all_eval("(i, els) => els.map(e => e.innerText || '')")
            .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// `locator.textContent()`.
    pub async fn text_content(&self) -> Result<Option<String>> {
        let v = self.read("(i, node) => node.textContent", &[]).await?;
        Ok(v.as_str().map(str::to_owned))
    }

    /// `locator.innerText()`.
    pub async fn inner_text(&self) -> Result<String> {
        let v = self
            .read(
                "(i, node) => { if (node.namespaceURI !== 'http://www.w3.org/1999/xhtml') \
                 throw i.createStacklessError('Node is not an HTMLElement'); return node.innerText; }",
                &[],
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.innerHTML()`.
    pub async fn inner_html(&self) -> Result<String> {
        let v = self.read("(i, node) => node.innerHTML", &[]).await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.getAttribute(name)`.
    pub async fn get_attribute(&self, name: &str) -> Result<Option<String>> {
        let v = self
            .read("(i, node, n) => node.getAttribute(n)", &[json!(name)])
            .await?;
        Ok(v.as_str().map(str::to_owned))
    }

    /// `locator.inputValue()`.
    pub async fn input_value(&self) -> Result<String> {
        let v = self
            .read(
                "(i, node) => { const e = i.retarget(node, 'follow-label'); \
                 if (!e || !['INPUT', 'TEXTAREA', 'SELECT'].includes(e.nodeName)) \
                 throw i.createStacklessError('Node is not an <input>, <textarea> or <select> element'); \
                 return e.value; }",
                &[],
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `locator.isVisible()`: right away (no waiting), like Playwright.
    pub async fn is_visible(&self) -> Result<bool> {
        let v = self
            .call(
                "(i, selector) => { const parsed = i.parseSelector(selector); \
                 const els = i.querySelectorAll(parsed, document); \
                 if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                 return els.length ? i.elementState(els[0], 'visible', true).matches : false; }",
                &[json!(self.selector)],
            )
            .await?;
        Ok(v == Value::Bool(true))
    }

    /// `locator.isHidden()`.
    pub async fn is_hidden(&self) -> Result<bool> {
        Ok(!self.is_visible().await?)
    }

    /// `locator.isChecked()` (waits for the element).
    pub async fn is_checked(&self) -> Result<bool> {
        let v = self
            .read(
                "(i, node) => { const s = i.elementState(node, 'checked', true); \
                 if (s.received === 'error:notcheckbox') throw i.createStacklessError('Not a checkbox or radio button'); \
                 return s.matches; }",
                &[],
            )
            .await?;
        Ok(v == Value::Bool(true))
    }

    /// Playwright's actionability for pointer actions: waits until the
    /// single element is in `states`, scrolls it into view if needed and
    /// checks the centre hits it; returns that point (viewport CSS px).
    pub(crate) async fn actionable_point(&self, states: &[&str]) -> Result<(f64, f64)> {
        const PREPARE: &str = "async (i, selector, states) => { const parsed = i.parseSelector(selector); \
             const els = i.querySelectorAll(parsed, document); \
             if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
             if (!els.length) return { retry: 'waiting for element' }; \
             const node = els[0]; \
             const state = await i.checkElementStates(node, states, true); \
             if (state === 'error:notconnected') return { retry: 'element is not attached' }; \
             if (state && state.missingState) return { retry: 'element is not ' + state.missingState }; \
             let box = node.getBoundingClientRect(); \
             if (box.top < 0 || box.left < 0 || box.bottom > innerHeight || box.right > innerWidth) { \
               node.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' }); \
               box = node.getBoundingClientRect(); } \
             const point = { x: box.left + box.width / 2, y: box.top + box.height / 2 }; \
             const hit = i.expectHitTarget(point, node); \
             if (hit !== 'done') return { retry: hit.hitTargetDescription + ' intercepts pointer events' }; \
             return { point }; }";
        let deadline = self.page.now() + self.page.timeout();
        let mut delay = Duration::from_millis(20);
        loop {
            self.page.run_locator_handlers().await;
            let (r, offset) = match self.scope().await? {
                Some((frame, offset)) => (
                    self.page
                        .injected_call_in(
                            frame.as_deref(),
                            PREPARE,
                            &[json!(self.selector), json!(states)],
                        )
                        .await?,
                    offset,
                ),
                None => (json!({"retry": "waiting for frame"}), (0.0, 0.0)),
            };
            if let Some(p) = r.get("point") {
                return Ok((
                    p["x"].as_f64().unwrap_or_default() + offset.0,
                    p["y"].as_f64().unwrap_or_default() + offset.1,
                ));
            }
            let last = r["retry"].as_str().unwrap_or_default();
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page.timeout().as_millis() as u64,
                    action: format!("waiting for locator('{}'): {last}", self.selector),
                });
            }
            self.page.sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(100));
        }
    }

    /// `locator.click()`.
    pub async fn click(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "click",
                json!({"selector": self.to_string(), "strict": true}),
                self.click_untraced(),
            )
            .await
    }

    async fn click_untraced(&self) -> Result<()> {
        let (x, y) = self
            .actionable_point(&["visible", "enabled", "stable"])
            .await?;
        self.page.click_at(x, y, 1).await
    }

    /// `locator.dblclick()`.
    pub async fn dblclick(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "dblclick",
                json!({"selector": self.to_string(), "strict": true}),
                self.dblclick_untraced(),
            )
            .await
    }

    async fn dblclick_untraced(&self) -> Result<()> {
        let (x, y) = self
            .actionable_point(&["visible", "enabled", "stable"])
            .await?;
        self.page.click_at(x, y, 2).await
    }

    /// `locator.hover()`.
    pub async fn hover(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "hover",
                json!({"selector": self.to_string(), "strict": true}),
                self.hover_untraced(),
            )
            .await
    }

    async fn hover_untraced(&self) -> Result<()> {
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        self.page.move_mouse(x, y).await
    }

    /// `locator.focus()`.
    pub async fn focus(&self) -> Result<()> {
        self.read("(i, node) => { i.focusNode(node); }", &[])
            .await
            .map(drop)
    }

    /// `locator.fill(value)`: Playwright's injected `fill`, then the text
    /// inserted (or the selection deleted for an empty value).
    pub async fn fill(&self, value: &str) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "fill",
                json!({"selector": self.to_string(), "strict": true, "value": value}),
                self.fill_untraced(value),
            )
            .await
    }

    async fn fill_untraced(&self, value: &str) -> Result<()> {
        self.page.run_locator_handlers().await;
        let r = self
            .read("(i, node, value) => i.fill(node, value)", &[json!(value)])
            .await?;
        match r.as_str() {
            Some("done") | None => Ok(()),
            Some("needsinput") => {
                if value.is_empty() {
                    self.page.press_shortcut("Delete").await
                } else {
                    self.page.insert_text(value).await
                }
            }
            Some(other) => Err(Error::Evaluation(other.to_owned())),
        }
    }

    /// `locator.press(key)`: focuses the element, then presses `key`.
    pub async fn press(&self, key: &str) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "press",
                json!({"selector": self.to_string(), "strict": true, "key": key}),
                self.press_untraced(key),
            )
            .await
    }

    async fn press_untraced(&self, key: &str) -> Result<()> {
        self.page.run_locator_handlers().await;
        self.focus().await?;
        self.page.press_shortcut(key).await
    }

    /// `locator.pressSequentially(text)`: focuses the element and types
    /// `text` key by key (characters off the keyboard are inserted).
    pub async fn press_sequentially(&self, text: &str) -> Result<()> {
        use crate::domain::input::{TypeStep, type_steps};
        self.focus().await?;
        for step in type_steps(text) {
            match step {
                TypeStep::Press(key) => self.page.press_shortcut(&key).await?,
                TypeStep::Insert(text) => self.page.insert_text(&text).await?,
            }
        }
        Ok(())
    }

    /// `locator.setChecked(checked)`.
    pub async fn set_checked(&self, checked: bool) -> Result<()> {
        if self.is_checked().await? == checked {
            return Ok(());
        }
        self.click().await?;
        if self.is_checked().await? != checked {
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
}
