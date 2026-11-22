//! Page-level selector shortcuts (`page.click(selector)` etc.): each one is
//! `page.locator(selector).<action>()`, strict like Playwright's.

use crate::application::locator::{Locator, WaitForState};
use crate::application::page::Page;
use crate::domain::actionability::Point;
use crate::domain::error::Result;

impl Page {
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
    /// Playwright's deprecated `page.type(selector, text)`.
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
    pub async fn set_input_files(
        &self,
        selector: &str,
        files: &[std::path::PathBuf],
    ) -> Result<()> {
        self.locator(selector).set_input_files(files).await
    }
    pub async fn focus(&self, selector: &str) -> Result<()> {
        self.locator(selector).focus().await
    }
    pub async fn dispatch_event(&self, selector: &str, kind: &str) -> Result<()> {
        self.locator(selector).dispatch_event(kind).await
    }
    pub async fn text_content(&self, selector: &str) -> Result<Option<String>> {
        self.locator(selector).text_content().await
    }
    pub async fn inner_text(&self, selector: &str) -> Result<String> {
        self.locator(selector).inner_text().await
    }
    pub async fn inner_html(&self, selector: &str) -> Result<String> {
        self.locator(selector).inner_html().await
    }
    pub async fn get_attribute(&self, selector: &str, name: &str) -> Result<Option<String>> {
        self.locator(selector).get_attribute(name).await
    }
    pub async fn input_value(&self, selector: &str) -> Result<String> {
        self.locator(selector).input_value().await
    }
    pub async fn is_visible(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_visible().await
    }
    pub async fn is_hidden(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_hidden().await
    }
    pub async fn is_enabled(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_enabled().await
    }
    pub async fn is_disabled(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_disabled().await
    }
    pub async fn is_editable(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_editable().await
    }
    pub async fn is_checked(&self, selector: &str) -> Result<bool> {
        self.locator(selector).is_checked().await
    }

    /// `page.waitForSelector(selector, { state })`; resolves with a locator
    /// for the element.
    pub async fn wait_for_selector(&self, selector: &str, state: WaitForState) -> Result<Locator> {
        let locator = self.locator(selector);
        locator.wait_for(state).await?;
        Ok(locator)
    }

    /// `page.dragAndDrop(source, target)`.
    pub async fn drag_and_drop(&self, source: &str, target: &str) -> Result<()> {
        self.locator(source).drag_to(&self.locator(target)).await
    }
}

impl Locator {
    /// `locator.dragTo(target)`: hover the source, press, hover the target, release.
    pub async fn drag_to(&self, target: &Locator) -> Result<()> {
        self.drag_to_with(target, None, None).await
    }

    /// `locator.dragTo(target, { sourcePosition, targetPosition })`.
    pub async fn drag_to_with(
        &self,
        target: &Locator,
        source_position: Option<Point>,
        target_position: Option<Point>,
    ) -> Result<()> {
        let mut hover = self.hover();
        if let Some(p) = source_position {
            hover = hover.position(p.x, p.y);
        }
        hover.await?;
        let mouse = self.page().mouse();
        mouse.down(crate::domain::input::MouseButton::Left).await?;
        let mut over = target.hover();
        if let Some(p) = target_position {
            over = over.position(p.x, p.y);
        }
        over.await?;
        mouse.up(crate::domain::input::MouseButton::Left).await
    }
}
