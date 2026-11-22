//! Frame-level Playwright API: selector actions, navigation of child frames,
//! waits and tags. The main frame delegates to the page.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};

use crate::application::frame::Frame;
use crate::application::handle::JsHandle;
use crate::application::locator::{Locator, WaitForState};
use crate::application::page::Response;
use crate::application::wait::wait_until;
use crate::domain::error::{Error, Result};
use crate::domain::frame::{LifecycleEvent, World};
use crate::domain::selectors::{self, TextMatch};
use crate::domain::url_matcher::UrlMatcher;

impl Frame {
    fn is_main(&self) -> bool {
        self.page().main_frame().id() == self.id()
    }

    pub async fn click(&self, selector: &str) -> Result<()> {
        self.locator(selector).click().await
    }
    pub async fn dblclick(&self, selector: &str) -> Result<()> {
        self.locator(selector).dblclick().await
    }
    pub async fn hover(&self, selector: &str) -> Result<()> {
        self.locator(selector).hover().await
    }
    pub async fn tap(&self, selector: &str) -> Result<()> {
        self.locator(selector).tap().await
    }
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
        self.locator(selector).fill(value).await
    }
    pub async fn press(&self, selector: &str, key: &str) -> Result<()> {
        self.locator(selector).press(key).await
    }
    /// `frame.type(selector, text)` (deprecated in Playwright).
    #[deprecated(note = "use locator(...).press_sequentially, like Playwright")]
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        self.locator(selector).press_sequentially(text).await
    }
    pub async fn check(&self, selector: &str) -> Result<()> {
        self.locator(selector).check().await
    }
    pub async fn uncheck(&self, selector: &str) -> Result<()> {
        self.locator(selector).uncheck().await
    }
    pub async fn set_checked(&self, selector: &str, checked: bool) -> Result<()> {
        self.locator(selector).set_checked(checked).await
    }
    pub async fn select_option(&self, selector: &str, values: &[&str]) -> Result<Vec<String>> {
        self.locator(selector).select_option(values).await
    }
    pub async fn set_input_files(&self, selector: &str, files: &[PathBuf]) -> Result<()> {
        self.locator(selector).set_input_files(files).await
    }
    pub async fn focus(&self, selector: &str) -> Result<()> {
        self.locator(selector).focus().await
    }
    pub async fn dispatch_event(&self, selector: &str, kind: &str) -> Result<()> {
        self.locator(selector).dispatch_event(kind).await
    }
    pub async fn drag_and_drop(&self, source: &str, target: &str) -> Result<()> {
        self.locator(source).drag_to(&self.locator(target)).await
    }
    pub async fn get_attribute(&self, selector: &str, name: &str) -> Result<Option<String>> {
        self.locator(selector).get_attribute(name).await
    }
    pub async fn inner_html(&self, selector: &str) -> Result<String> {
        self.locator(selector).inner_html().await
    }
    pub async fn inner_text(&self, selector: &str) -> Result<String> {
        self.locator(selector).inner_text().await
    }
    pub async fn text_content(&self, selector: &str) -> Result<Option<String>> {
        self.locator(selector).text_content().await
    }
    pub async fn input_value(&self, selector: &str) -> Result<String> {
        self.locator(selector).input_value().await
    }
    pub async fn is_checked(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_checked().await
    }
    pub async fn is_disabled(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_disabled().await
    }
    pub async fn is_editable(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_editable().await
    }
    pub async fn is_enabled(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_enabled().await
    }
    pub async fn is_hidden(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_hidden().await
    }
    pub async fn is_visible(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_visible().await
    }
    pub fn get_by_alt_text(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_alt_text(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }
    pub fn get_by_title(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_title(&text.into(), exact))
    }

    /// `frame.waitForSelector(selector, {state})`.
    pub async fn wait_for_selector(&self, selector: &str, state: WaitForState) -> Result<()> {
        self.locator(selector).wait_for(state).await
    }

    /// `frame.waitForTimeout(ms)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.page().wait_for_timeout(duration).await
    }

    /// `frame.waitForFunction(fn)`: until it returns a truthy value.
    pub async fn wait_for_function(&self, source: &str) -> Result<Value> {
        let clock = self.page().inner.shared.services.clock.clone();
        let deadline = clock.now_ms() + self.page().default_timeout().as_millis() as u64;
        loop {
            let v: Value = self.evaluate(source).await?;
            if !matches!(v, Value::Null | Value::Bool(false)) && v != json!(0) && v != json!("") {
                return Ok(v);
            }
            if clock.now_ms() >= deadline {
                return Err(Error::Timeout {
                    ms: self.page().default_timeout().as_millis() as u64,
                    action: "waiting for function".into(),
                });
            }
            clock.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `frame.evaluateHandle(fn)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        let page = self.page();
        let context = page
            .frame_context(Some(self.id()), World::Main, page.default_timeout())
            .await?;
        let r = page
            .send(
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": crate::application::js::function_declaration(source),
                       "executionContextId": context, "arguments": [{"value": null}],
                       "returnByValue": false, "awaitPromise": true}),
            )
            .await?;
        crate::application::js::result_value(&r)?;
        let mut h = JsHandle::from_remote(page, &r["result"]);
        h.frame = (!self.is_main()).then(|| self.id().to_owned());
        Ok(h)
    }

    /// `frame.addStyleTag({content})`.
    pub async fn add_style_tag_content(&self, css: &str) -> Result<()> {
        self.evaluate_with::<Value, _>(
            "css => new Promise((res, rej) => { const s = document.createElement('style'); s.appendChild(document.createTextNode(css)); s.onload = res; s.onerror = rej; document.head.appendChild(s); res(); })",
            css,
        )
        .await?;
        Ok(())
    }

    /// `frame.addScriptTag({content})`.
    pub async fn add_script_tag_content(&self, js: &str) -> Result<()> {
        self.evaluate_with::<Value, _>(
            "js => { const s = document.createElement('script'); s.type = 'text/javascript'; s.text = js; document.head.appendChild(s); }",
            js,
        )
        .await?;
        Ok(())
    }

    /// `frame.frameElement()`: the `<iframe>` element hosting this frame.
    pub async fn frame_element(&self) -> Result<JsHandle> {
        let page = self.page();
        let owner = page
            .send("DOM.getFrameOwner", json!({"frameId": self.id()}))
            .await?;
        let backend = owner.get("backendNodeId").cloned().unwrap_or(Value::Null);
        let parent = self.parent_frame();
        let context = page
            .frame_context(
                parent.as_ref().map(|p| p.id()),
                World::Main,
                page.default_timeout(),
            )
            .await?;
        let r = page
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": backend, "executionContextId": context}),
            )
            .await?;
        let mut h = JsHandle::from_remote(page, &r["object"]);
        h.frame = parent.filter(|p| !p.is_main()).map(|p| p.id().to_owned());
        Ok(h)
    }

    /// Waits for this frame's lifecycle `state`.
    pub async fn wait_for_load_state(&self, state: LifecycleEvent) -> Result<()> {
        let inner = &self.page().inner;
        let id = self.id().to_owned();
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            self.page().navigation_timeout(),
            "waiting for load state",
            || {
                if inner.closed.load(Ordering::SeqCst) {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                let Some(f) = st.tree.frame(&id) else {
                    return Some(Err(Error::TargetClosed("frame has been detached".into())));
                };
                f.has(state).then_some(Ok(()))
            },
        )
        .await
    }

    /// `frame.waitForURL(url)`.
    pub async fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let matcher = url.into();
        let inner = &self.page().inner;
        let id = self.id().to_owned();
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            self.page().navigation_timeout(),
            "waiting for URL",
            || {
                let st = inner.state.lock().unwrap();
                let f = st.tree.frame(&id)?;
                (matcher.matches(&f.url) && f.has(LifecycleEvent::Load)).then_some(Ok(()))
            },
        )
        .await
    }

    /// `frame.goto(url)`: navigates this frame and waits for `load`.
    pub async fn goto(&self, url: &str) -> Result<Option<Response>> {
        if self.is_main() {
            return self.page().goto(url).await;
        }
        let page = self.page();
        let r = page
            .send("Page.navigate", json!({"url": url, "frameId": self.id()}))
            .await?;
        if let Some(e) = r.get("errorText").and_then(Value::as_str) {
            return Err(Error::Navigation(format!("{e} at {url}")));
        }
        let loader = r.get("loaderId").and_then(Value::as_str).map(str::to_owned);
        let inner = &page.inner;
        let id = self.id().to_owned();
        wait_until(
            &*inner.shared.services.clock,
            &inner.changed,
            page.navigation_timeout(),
            "navigating the frame",
            || {
                let st = inner.state.lock().unwrap();
                let f = st.tree.frame(&id)?;
                if loader.is_some() && f.loader_id != loader {
                    return None;
                }
                if !f.has(LifecycleEvent::Load) {
                    return None;
                }
                Some(Ok(loader
                    .as_ref()
                    .and_then(|l| st.responses.get(l))
                    .cloned()))
            },
        )
        .await
    }

    /// `frame.setContent(html)`.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        if self.is_main() {
            return self.page().set_content(html).await;
        }
        self.page()
            .call_function_in(
                Some(self.id()),
                World::Utility,
                "html => { document.open(); document.write(html); document.close(); }",
                &[json!(html)],
                self.page().default_timeout(),
                "setting content",
            )
            .await?;
        self.wait_for_load_state(LifecycleEvent::Load).await
    }

    /// `frame.waitForNavigation()`: the main frame's navigation (see the page).
    pub fn wait_for_navigation(&self) -> crate::application::extras::WaitForNavigation<'_> {
        self.page().wait_for_navigation()
    }
}
