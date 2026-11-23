//! Locators on the Firefox engine: Playwright's injected script in a BiDi
//! sandbox (Firefox's equivalent of Chromium's utility world).

use std::time::Duration;

use serde_json::{Value, json};

use crate::application::firefox::Page;
use crate::domain::error::{Error, Result};
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
