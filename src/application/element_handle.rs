//! Element handles (Playwright's `ElementHandle`): a `JsHandle` to a DOM
//! element. Actions go through the locator machinery (auto-waiting and
//! actionability) by addressing the element with an internal selector engine.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use crate::application::frame::Frame;
use crate::application::handle::JsHandle;
use crate::application::locator::{BoundingBox, Locator, WaitForState};
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

impl JsHandle {
    /// A locator that resolves exactly this element.
    pub async fn as_locator(&self) -> Result<Locator> {
        let page = self.page().clone();
        if let Some(id) = self.registered.lock().unwrap().clone() {
            return Ok(Locator::in_frame(
                &page,
                self.frame.clone(),
                format!("chromiumoxide-handle={id}"),
            )
            .ignoring_visibility());
        }
        let object_id = self
            .object_id()
            .ok_or_else(|| Error::InvalidArgument("JSHandle is not an element".into()))?;
        let node = page
            .send("DOM.describeNode", json!({"objectId": object_id}))
            .await?;
        let backend = node
            .pointer("/node/backendNodeId")
            .cloned()
            .ok_or_else(|| Error::InvalidArgument("JSHandle is not an element".into()))?;
        // Make sure the injected script (and its engine) exists in the frame.
        let probe =
            Locator::in_frame(&page, self.frame.clone(), ":root".into()).ignoring_visibility();
        probe.count().await?;
        let context = page
            .frame_context(
                self.frame.as_deref(),
                World::Utility,
                page.default_timeout(),
            )
            .await?;
        let adopted = page
            .send(
                "DOM.resolveNode",
                json!({"backendNodeId": backend, "executionContextId": context}),
            )
            .await?;
        let utility_object = adopted
            .pointer("/object/objectId")
            .cloned()
            .unwrap_or(Value::Null);
        let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst).to_string();
        page.send(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": "function(id) { (globalThis.__chromiumoxide_handles ||= new Map()).set(id, this); }",
                "objectId": utility_object,
                "arguments": [{"value": id}],
            }),
        )
        .await?;
        *self.registered.lock().unwrap() = Some(id.clone());
        Ok(Locator::in_frame(
            &page,
            self.frame.clone(),
            format!("chromiumoxide-handle={id}"),
        )
        .ignoring_visibility())
    }

    pub async fn click(&self) -> Result<()> {
        self.as_locator().await?.click().await
    }
    pub async fn dblclick(&self) -> Result<()> {
        self.as_locator().await?.dblclick().await
    }
    pub async fn hover(&self) -> Result<()> {
        self.as_locator().await?.hover().await
    }
    pub async fn tap(&self) -> Result<()> {
        self.as_locator().await?.tap().await
    }
    pub async fn fill(&self, value: &str) -> Result<()> {
        self.as_locator().await?.fill(value).await
    }
    pub async fn press(&self, key: &str) -> Result<()> {
        self.as_locator().await?.press(key).await
    }
    /// `elementHandle.type(text)` (deprecated in Playwright).
    #[deprecated(note = "use press_sequentially, like Playwright")]
    pub async fn type_text(&self, text: &str) -> Result<()> {
        self.as_locator().await?.press_sequentially(text).await
    }
    pub async fn press_sequentially(&self, text: &str) -> Result<()> {
        self.as_locator().await?.press_sequentially(text).await
    }
    pub async fn check(&self) -> Result<()> {
        self.as_locator().await?.check().await
    }
    pub async fn uncheck(&self) -> Result<()> {
        self.as_locator().await?.uncheck().await
    }
    pub async fn set_checked(&self, checked: bool) -> Result<()> {
        self.as_locator().await?.set_checked(checked).await
    }
    pub async fn select_option(&self, values: &[&str]) -> Result<Vec<String>> {
        self.as_locator().await?.select_option(values).await
    }
    pub async fn set_input_files(&self, files: &[PathBuf]) -> Result<()> {
        self.as_locator().await?.set_input_files(files).await
    }
    pub async fn focus(&self) -> Result<()> {
        self.as_locator().await?.focus().await
    }
    pub async fn dispatch_event(&self, kind: &str) -> Result<()> {
        self.as_locator().await?.dispatch_event(kind).await
    }
    pub async fn scroll_into_view_if_needed(&self) -> Result<()> {
        self.as_locator().await?.scroll_into_view_if_needed().await
    }
    pub async fn select_text(&self) -> Result<()> {
        self.as_locator().await?.select_text().await
    }
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        self.as_locator().await?.screenshot().await
    }
    pub async fn bounding_box(&self) -> Result<Option<BoundingBox>> {
        self.as_locator().await?.bounding_box().await
    }
    pub async fn get_attribute(&self, name: &str) -> Result<Option<String>> {
        self.as_locator().await?.get_attribute(name).await
    }
    pub async fn inner_html(&self) -> Result<String> {
        self.as_locator().await?.inner_html().await
    }
    pub async fn inner_text(&self) -> Result<String> {
        self.as_locator().await?.inner_text().await
    }
    pub async fn text_content(&self) -> Result<Option<String>> {
        self.as_locator().await?.text_content().await
    }
    pub async fn input_value(&self) -> Result<String> {
        self.as_locator().await?.input_value().await
    }
    pub async fn is_checked(&self) -> Result<bool> {
        self.as_locator().await?.is_checked().await
    }
    pub async fn is_disabled(&self) -> Result<bool> {
        self.as_locator().await?.is_disabled().await
    }
    pub async fn is_editable(&self) -> Result<bool> {
        self.as_locator().await?.is_editable().await
    }
    pub async fn is_enabled(&self) -> Result<bool> {
        self.as_locator().await?.is_enabled().await
    }
    pub async fn is_hidden(&self) -> Result<bool> {
        self.as_locator().await?.is_hidden().await
    }
    pub async fn is_visible(&self) -> Result<bool> {
        self.as_locator().await?.is_visible().await
    }

    /// `elementHandle.waitForElementState(state)`: `visible`, `hidden`,
    /// `stable`, `enabled`, `disabled` or `editable`.
    pub async fn wait_for_element_state(&self, state: &str) -> Result<()> {
        self.as_locator().await?.wait_for_states(&[state]).await
    }

    /// `elementHandle.waitForSelector(selector)`: a descendant, once visible.
    pub async fn wait_for_selector(&self, selector: &str) -> Result<Option<JsHandle>> {
        let inner = self.as_locator().await?.locator(selector);
        inner.wait_for(WaitForState::Visible).await?;
        Ok(Some(inner.element_handle().await?))
    }

    /// `elementHandle.contentFrame()`: the frame of an `<iframe>`.
    pub async fn content_frame(&self) -> Result<Option<Frame>> {
        let Some(object_id) = self.object_id() else {
            return Ok(None);
        };
        let node = self
            .page()
            .send("DOM.describeNode", json!({"objectId": object_id}))
            .await?;
        Ok(node
            .pointer("/node/frameId")
            .and_then(Value::as_str)
            .map(|id| self.page().frame_by_id(id)))
    }

    /// `elementHandle.ownerFrame()`: the frame the element lives in.
    pub async fn owner_frame(&self) -> Result<Option<Frame>> {
        Ok(Some(match &self.frame {
            Some(id) => self.page().frame_by_id(id),
            None => self.page().main_frame(),
        }))
    }
}
