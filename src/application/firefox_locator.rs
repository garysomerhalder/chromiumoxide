//! Locators on the Firefox engine: Playwright's injected script in a BiDi
//! sandbox (Firefox's equivalent of Chromium's utility world).

use std::time::Duration;

use serde_json::{Value, json};

use crate::application::firefox::Page;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::webdriver_key;
use crate::domain::input::{resolve_smart_modifier, split_shortcut};
use crate::domain::selectors::{self, RoleOptions, TextMatch, compose};

/// `page.locator(selector)` and the `get_by_*` builders on a Firefox page.
impl Page {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.clone(),
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
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }
}

/// A Firefox locator: a selector resolved lazily, strict like Playwright's.
#[derive(Clone, Debug)]
pub struct Locator {
    page: Page,
    selector: String,
}

/// Default action timeout (Playwright's 30 s).
const TIMEOUT: Duration = Duration::from_secs(30);

impl Locator {
    fn with(&self, selector: String) -> Locator {
        Locator {
            page: self.page.clone(),
            selector,
        }
    }
    /// `locator.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        self.with(compose::chain(&self.selector, selector))
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
    pub fn get_by_role(&self, role: &str, options: RoleOptions) -> Locator {
        self.locator(&selectors::get_by_role(role, &options))
    }
    pub fn get_by_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_text(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }

    /// The selector string (Playwright's internal form).
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Runs `body(injected, element)` on the single element the locator
    /// resolves to, waiting until there is one (strict: more than one is an
    /// error).
    async fn read(&self, body: &str, args: &[Value]) -> Result<Value> {
        let function = format!(
            "(i, selector, args) => {{ const parsed = i.parseSelector(selector); \
             const els = i.querySelectorAll(parsed, document); \
             if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
             if (!els.length) return {{ found: false }}; \
             return {{ found: true, value: ({body})(i, els[0], ...args) }}; }}"
        );
        let deadline = self.page.now() + TIMEOUT;
        let mut delay = Duration::from_millis(20);
        loop {
            let r = self
                .page
                .injected_call(&function, &[json!(self.selector), json!(args)])
                .await?;
            if r["found"] == Value::Bool(true) {
                return Ok(r["value"].clone());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for locator('{}')", self.selector),
                });
            }
            self.page.sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(100));
        }
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

    /// Playwright's actionability for pointer actions: waits until the
    /// single element is in `states`, scrolls it into view if needed and
    /// checks the centre hits it; returns that point (viewport CSS px).
    async fn actionable_point(&self, states: &[&str]) -> Result<(f64, f64)> {
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
        let deadline = self.page.now() + TIMEOUT;
        let mut delay = Duration::from_millis(20);
        loop {
            let r = self
                .page
                .injected_call(PREPARE, &[json!(self.selector), json!(states)])
                .await?;
            if let Some(p) = r.get("point") {
                return Ok((
                    p["x"].as_f64().unwrap_or_default(),
                    p["y"].as_f64().unwrap_or_default(),
                ));
            }
            let last = r["retry"].as_str().unwrap_or_default();
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for locator('{}'): {last}", self.selector),
                });
            }
            self.page.sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(100));
        }
    }

    fn pointer(actions: Vec<Value>) -> Value {
        json!([{"type": "pointer", "id": "__chromiumoxide_mouse",
                "parameters": {"pointerType": "mouse"}, "actions": actions}])
    }

    /// `locator.click()`.
    pub async fn click(&self) -> Result<()> {
        let (x, y) = self
            .actionable_point(&["visible", "enabled", "stable"])
            .await?;
        self.page
            .perform_actions(Self::pointer(vec![
                json!({"type": "pointerMove", "x": x.round() as i64, "y": y.round() as i64}),
                json!({"type": "pointerDown", "button": 0}),
                json!({"type": "pointerUp", "button": 0}),
            ]))
            .await
    }

    /// `locator.hover()`.
    pub async fn hover(&self) -> Result<()> {
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        self.page
            .perform_actions(Self::pointer(vec![json!({
                "type": "pointerMove", "x": x.round() as i64, "y": y.round() as i64
            })]))
            .await
    }

    /// `locator.focus()`.
    pub async fn focus(&self) -> Result<()> {
        self.read("(i, node) => { i.focusNode(node); }", &[])
            .await
            .map(drop)
    }

    /// `locator.fill(value)`: Playwright's injected `fill`, then the text
    /// typed in (or the selection deleted for an empty value).
    pub async fn fill(&self, value: &str) -> Result<()> {
        let r = self
            .read("(i, node, value) => i.fill(node, value)", &[json!(value)])
            .await?;
        match r.as_str() {
            Some("done") => Ok(()),
            Some("needsinput") => {
                if value.is_empty() {
                    self.press_keys("Delete").await
                } else {
                    self.page.insert_text(value).await
                }
            }
            Some(other) => Err(Error::Evaluation(other.to_owned())),
            None => Ok(()),
        }
    }

    async fn press_keys(&self, shortcut: &str) -> Result<()> {
        let keys: Vec<String> = split_shortcut(shortcut)
            .iter()
            .map(|k| resolve_smart_modifier(k, cfg!(target_os = "macos")))
            .map(|k| {
                webdriver_key(&k)
                    .ok_or_else(|| Error::InvalidArgument(format!("Unknown key: \"{k}\"")))
            })
            .collect::<Result<_>>()?;
        let mut actions: Vec<Value> = keys
            .iter()
            .map(|k| json!({"type": "keyDown", "value": k}))
            .collect();
        actions.extend(
            keys.iter()
                .rev()
                .map(|k| json!({"type": "keyUp", "value": k})),
        );
        self.page
            .perform_actions(
                json!([{"type": "key", "id": "__chromiumoxide_keyboard", "actions": actions}]),
            )
            .await
    }

    /// `locator.press(key)`: focuses the element, then presses `key`
    /// (`"Enter"`, `"Control+a"`, ...).
    pub async fn press(&self, key: &str) -> Result<()> {
        self.focus().await?;
        self.press_keys(key).await
    }

    /// `locator.pressSequentially(text)`: focuses the element and types
    /// `text` key by key.
    pub async fn press_sequentially(&self, text: &str) -> Result<()> {
        self.focus().await?;
        let mut actions = Vec::new();
        for c in text.chars() {
            let value = if c == '\n' {
                "\u{E007}".to_owned()
            } else {
                c.to_string()
            };
            actions.push(json!({"type": "keyDown", "value": value}));
            actions.push(json!({"type": "keyUp", "value": value}));
        }
        self.page
            .perform_actions(
                json!([{"type": "key", "id": "__chromiumoxide_keyboard", "actions": actions}]),
            )
            .await
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

    async fn set_checked(&self, checked: bool) -> Result<()> {
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

    /// `locator.isVisible()` (does not wait).
    pub async fn is_visible(&self) -> Result<bool> {
        let v = self
            .page
            .injected_call(
                "(i, selector) => { const parsed = i.parseSelector(selector); \
                 const els = i.querySelectorAll(parsed, document); \
                 if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                 return els.length ? i.elementState(els[0], 'visible', true).matches : false; }",
                &[json!(self.selector)],
            )
            .await?;
        Ok(v == Value::Bool(true))
    }

    /// `locator.isHidden()` (does not wait).
    pub async fn is_hidden(&self) -> Result<bool> {
        Ok(!self.is_visible().await?)
    }

    /// `locator.count()` (does not wait).
    pub async fn count(&self) -> Result<usize> {
        let v = self
            .page
            .injected_call(
                "(i, selector) => i.querySelectorAll(i.parseSelector(selector), document).length",
                &[json!(self.selector)],
            )
            .await?;
        Ok(v.as_u64().unwrap_or_default() as usize)
    }

    async fn all_strings(&self, map: &str) -> Result<Vec<String>> {
        let v = self
            .page
            .injected_call(
                &format!(
                    "(i, selector) => i.querySelectorAll(i.parseSelector(selector), document).map({map})"
                ),
                &[json!(self.selector)],
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

    /// `locator.allTextContents()`.
    pub async fn all_text_contents(&self) -> Result<Vec<String>> {
        self.all_strings("e => e.textContent || ''").await
    }

    /// `locator.allInnerTexts()`.
    pub async fn all_inner_texts(&self) -> Result<Vec<String>> {
        self.all_strings("e => e.innerText").await
    }
}
