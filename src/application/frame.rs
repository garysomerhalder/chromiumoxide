//! Frames: a page's main frame and its (same-process) iframes.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::js;
use crate::application::locator::{FrameLocator, Locator};
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;
use crate::domain::selectors::{self, RoleOptions, TextMatch};

/// A frame of a page (Playwright's `Frame`). Cheap to clone.
#[derive(Clone)]
pub struct Frame {
    page: Page,
    id: String,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Frame")
            .field("id", &self.id)
            .field("url", &self.url())
            .finish()
    }
}

impl Page {
    /// `page.mainFrame()`.
    pub fn main_frame(&self) -> Frame {
        let id = self.main_frame_id().unwrap_or_default();
        Frame {
            page: self.clone(),
            id,
        }
    }

    /// `page.frames()`: the main frame and its descendants.
    pub fn frames(&self) -> Vec<Frame> {
        let st = self.inner.state.lock().unwrap();
        st.tree
            .frames()
            .map(|f| Frame {
                page: self.clone(),
                id: f.id.clone(),
            })
            .collect()
    }

    /// `page.frame(name)`: a frame by its `name` attribute, else by URL.
    pub fn frame(&self, name_or_url: &str) -> Option<Frame> {
        let st = self.inner.state.lock().unwrap();
        st.tree
            .frames()
            .find(|f| f.name == name_or_url)
            .or_else(|| st.tree.frames().find(|f| f.url == name_or_url))
            .map(|f| Frame {
                page: self.clone(),
                id: f.id.clone(),
            })
    }
}

impl Frame {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    fn with<T>(&self, f: impl FnOnce(&crate::domain::frame::Frame) -> T) -> Option<T> {
        let st = self.page.inner.state.lock().unwrap();
        st.tree.frame(&self.id).map(f)
    }

    pub fn url(&self) -> String {
        self.with(|f| f.url.clone()).unwrap_or_default()
    }

    pub fn name(&self) -> String {
        self.with(|f| f.name.clone()).unwrap_or_default()
    }

    pub fn is_detached(&self) -> bool {
        self.with(|_| ()).is_none()
    }

    pub fn parent_frame(&self) -> Option<Frame> {
        let parent = self.with(|f| f.parent_id.clone()).flatten()?;
        Some(Frame {
            page: self.page.clone(),
            id: parent,
        })
    }

    pub fn child_frames(&self) -> Vec<Frame> {
        let st = self.page.inner.state.lock().unwrap();
        st.tree
            .children(&self.id)
            .into_iter()
            .map(|f| Frame {
                page: self.page.clone(),
                id: f.id.clone(),
            })
            .collect()
    }

    async fn call(
        &self,
        world: World,
        declaration: &str,
        args: &[Value],
        action: &str,
    ) -> Result<Value> {
        self.page
            .call_function_in(
                Some(&self.id),
                world,
                declaration,
                args,
                self.page.default_timeout(),
                action,
            )
            .await
    }

    /// `frame.evaluate(expression | function)`.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self
            .call(
                World::Main,
                &js::function_declaration(source),
                &[],
                "evaluating",
            )
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `frame.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self
            .call(
                World::Main,
                &js::function_declaration(source),
                &[arg],
                "evaluating",
            )
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `frame.title()`.
    pub async fn title(&self) -> Result<String> {
        let v = self
            .call(
                World::Utility,
                "function() { return document.title }",
                &[],
                "reading the title",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `frame.content()`.
    pub async fn content(&self) -> Result<String> {
        let v = self
            .call(
                World::Utility,
                "function() { let h = ''; if (document.doctype) h = new XMLSerializer().serializeToString(document.doctype); if (document.documentElement) h += document.documentElement.outerHTML; return h; }",
                &[],
                "reading content",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `frame.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator::in_frame(&self.page, Some(self.id.clone()), selector.to_owned())
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
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }
    /// `frame.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        self.locator(selector).content_frame()
    }
}
