//! Frames on the WebKit engine: `page.frameLocator()` (locators that run in
//! an iframe's document) and `page.frames()`. An iframe element's frame id
//! comes from `DOM.describeNode` (`contentFrameId`), as in Playwright's
//! `WKPage.getContentFrame`.

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::webkit::Page;
use crate::application::webkit_locator::Locator;
use crate::domain::error::{Error, Result};
use crate::domain::selectors::{self, RoleOptions, TextMatch};

/// How long a found iframe may take to report its execution contexts.
const FRAME_READY: Duration = Duration::from_secs(5);

/// `page.frameLocator(selector)`: locators inside an iframe.
#[derive(Clone, Debug)]
pub struct FrameLocator {
    page: Page,
    frames: Vec<String>,
}

impl FrameLocator {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            selector: selector.to_owned(),
            frames: self.frames.clone(),
        }
    }
    /// A frame inside this frame.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        let mut frames = self.frames.clone();
        frames.push(selector.to_owned());
        FrameLocator {
            page: self.page.clone(),
            frames,
        }
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
}

/// A frame of a WebKit page (`page.frames()`).
#[derive(Clone, Debug)]
pub struct Frame {
    page: Page,
    id: String,
    url: String,
    main: bool,
}

impl Frame {
    /// The frame's URL when it was listed.
    pub fn url(&self) -> &str {
        &self.url
    }
    /// Whether this is the page's main frame.
    pub fn is_main(&self) -> bool {
        self.main
    }
    /// `frame.evaluate(expression or function)` in the frame's main world.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let frame = (!self.main).then_some(self.id.as_str());
        let v = self.page.evaluate_value_in(source, None, frame).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }
}

impl Page {
    /// `page.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.clone(),
            frames: vec![selector.to_owned()],
        }
    }

    /// `page.frames()`: the main frame first, then its descendants in
    /// document order (`Page.getResourceTree`).
    pub async fn frames(&self) -> Result<Vec<Frame>> {
        let tree = self.target_call("Page.getResourceTree", json!({})).await?;
        let mut out = Vec::new();
        fn walk(page: &Page, node: &Value, main: bool, out: &mut Vec<Frame>) {
            let f = &node["frame"];
            out.push(Frame {
                page: page.clone(),
                id: f["id"].as_str().unwrap_or_default().to_owned(),
                url: f["url"].as_str().unwrap_or_default().to_owned(),
                main,
            });
            for child in node["childFrames"].as_array().into_iter().flatten() {
                walk(page, child, false, out);
            }
        }
        walk(self, &tree["frameTree"], true, &mut out);
        Ok(out)
    }

    /// The frame of the single iframe `selector` matches in `parent` (`None`:
    /// the main frame) and its content box's offset in `parent`'s viewport;
    /// `None` while there is no such iframe yet.
    pub(crate) async fn child_frame(
        &self,
        parent: Option<&str>,
        selector: &str,
    ) -> Result<Option<(String, (f64, f64))>> {
        let found = self
            .injected_call_in(
                parent,
                "(i, selector) => { const parsed = i.parseSelector(selector); \
                 const els = i.querySelectorAll(parsed, document); \
                 if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                 if (!els.length) return { found: false }; \
                 const el = els[0], box = el.getBoundingClientRect(), s = getComputedStyle(el); \
                 globalThis.__chromiumoxide_frame_element = el; \
                 return { found: true, \
                   x: box.left + parseFloat(s.borderLeftWidth) + parseFloat(s.paddingLeft), \
                   y: box.top + parseFloat(s.borderTopWidth) + parseFloat(s.paddingTop) }; }",
                &[json!(selector)],
            )
            .await?;
        if found["found"] != Value::Bool(true) {
            return Ok(None);
        }
        let element = self
            .run_in(
                "(() => { const el = globalThis.__chromiumoxide_frame_element; \
                 delete globalThis.__chromiumoxide_frame_element; return el; })()",
                parent,
                true,
            )
            .await?;
        let Some(object) = element["objectId"].as_str() else {
            return Ok(None);
        };
        let node = self
            .target_call("DOM.describeNode", json!({"objectId": object}))
            .await?;
        let Some(frame) = node["contentFrameId"].as_str().map(str::to_owned) else {
            return Ok(None);
        };
        let deadline = self.now() + FRAME_READY;
        while self.context_of(Some(&frame), true).is_none() {
            if self.now() >= deadline {
                return Ok(None);
            }
            self.sleep(Duration::from_millis(20)).await;
        }
        Ok(Some((
            frame,
            (
                found["x"].as_f64().unwrap_or_default(),
                found["y"].as_f64().unwrap_or_default(),
            ),
        )))
    }
}
