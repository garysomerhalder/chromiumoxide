//! Frames on the Firefox engine: Playwright's `Frame` over a BiDi browsing
//! context, and Playwright's selector shortcuts (`page.click(selector)`,
//! `frame.fill(selector, value)`, ...) on pages and frames.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::actions::StyleTag;
use crate::application::firefox::{Frame, Page, UTILITY};
use crate::application::firefox_locator::{FrameLocator, Locator};
use crate::application::scripts::{ADD_SCRIPT, ScriptTag};
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::selectors::{self, RoleOptions, TextMatch};
use crate::domain::url_matcher::UrlMatcher;

/// Playwright's default timeout for waits.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Playwright's selector shortcuts: each is `self.locator(selector).<method>`.
macro_rules! selector_shortcuts {
    () => {
        /// `click(selector)`.
        pub async fn click(&self, selector: &str) -> Result<()> {
            self.locator(selector).click().await
        }
        /// `dblclick(selector)`.
        pub async fn dblclick(&self, selector: &str) -> Result<()> {
            self.locator(selector).dblclick().await
        }
        /// `tap(selector)`.
        pub async fn tap(&self, selector: &str) -> Result<()> {
            self.locator(selector).tap().await
        }
        /// `check(selector)`.
        pub async fn check(&self, selector: &str) -> Result<()> {
            self.locator(selector).check().await
        }
        /// `uncheck(selector)`.
        pub async fn uncheck(&self, selector: &str) -> Result<()> {
            self.locator(selector).uncheck().await
        }
        /// `setChecked(selector, checked)`.
        pub async fn set_checked(&self, selector: &str, checked: bool) -> Result<()> {
            self.locator(selector).set_checked(checked).await
        }
        /// `focus(selector)`.
        pub async fn focus(&self, selector: &str) -> Result<()> {
            self.locator(selector).focus().await
        }
        /// `hover(selector)`.
        pub async fn hover(&self, selector: &str) -> Result<()> {
            self.locator(selector).hover().await
        }
        /// `fill(selector, value)`.
        pub async fn fill(&self, selector: &str, value: &str) -> Result<()> {
            self.locator(selector).fill(value).await
        }
        /// `press(selector, key)`.
        pub async fn press(&self, selector: &str, key: &str) -> Result<()> {
            self.locator(selector).press(key).await
        }
        /// `type(selector, text)` (deprecated in Playwright: use
        /// `locator(selector).press_sequentially(text)`).
        #[deprecated(note = "use locator(selector).press_sequentially, like Playwright")]
        pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
            self.locator(selector).press_sequentially(text).await
        }
        /// `dispatchEvent(selector, type)`.
        pub async fn dispatch_event(&self, selector: &str, kind: &str) -> Result<()> {
            self.locator(selector).dispatch_event(kind).await
        }
        /// `selectOption(selector, values)`.
        pub async fn select_option(&self, selector: &str, values: &[&str]) -> Result<Vec<String>> {
            self.locator(selector).select_option(values).await
        }
        /// `setInputFiles(selector, files)`.
        pub async fn set_input_files(&self, selector: &str, files: &[PathBuf]) -> Result<()> {
            self.locator(selector).set_input_files(files).await
        }
        /// `dragAndDrop(source, target)`.
        pub async fn drag_and_drop(&self, source: &str, target: &str) -> Result<()> {
            self.locator(source).drag_to(&self.locator(target)).await
        }
        /// `getAttribute(selector, name)`.
        pub async fn get_attribute(&self, selector: &str, name: &str) -> Result<Option<String>> {
            self.locator(selector).get_attribute(name).await
        }
        /// `textContent(selector)`.
        pub async fn text_content(&self, selector: &str) -> Result<Option<String>> {
            self.locator(selector).text_content().await
        }
        /// `innerText(selector)`.
        pub async fn inner_text(&self, selector: &str) -> Result<String> {
            self.locator(selector).inner_text().await
        }
        /// `innerHTML(selector)`.
        pub async fn inner_html(&self, selector: &str) -> Result<String> {
            self.locator(selector).inner_html().await
        }
        /// `inputValue(selector)`.
        pub async fn input_value(&self, selector: &str) -> Result<String> {
            self.locator(selector).input_value().await
        }
        /// `isChecked(selector)`.
        pub async fn is_checked(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_checked().await
        }
        /// `isDisabled(selector)`.
        pub async fn is_disabled(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_disabled().await
        }
        /// `isEditable(selector)`.
        pub async fn is_editable(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_editable().await
        }
        /// `isEnabled(selector)`.
        pub async fn is_enabled(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_enabled().await
        }
        /// `isHidden(selector)` (does not wait).
        pub async fn is_hidden(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_hidden().await
        }
        /// `isVisible(selector)` (does not wait).
        pub async fn is_visible(&self, selector: &str) -> Result<bool> {
            self.locator(selector).is_visible().await
        }
    };
}
#[allow(unused_imports)]
pub(crate) use selector_shortcuts;

impl Page {
    selector_shortcuts!();

    /// `page.mainFrame()`.
    pub fn main_frame(&self) -> Frame {
        Frame {
            page: self.clone(),
            context: self.main_context(),
            url: self.url(),
            parent: None,
        }
    }

    /// `page.frame(name)`: the frame with that `name` (the main frame's
    /// name is empty).
    pub async fn frame(&self, name: &str) -> Result<Option<Frame>> {
        for frame in self.frames().await? {
            if frame.name().await.unwrap_or_default() == name {
                return Ok(Some(frame));
            }
        }
        Ok(None)
    }

    /// `page.frame({ url })`: the first frame whose URL matches.
    pub async fn frame_by_url(&self, url: impl Into<UrlMatcher>) -> Result<Option<Frame>> {
        let matcher = url.into();
        Ok(self
            .frames()
            .await?
            .into_iter()
            .find(|f| matcher.matches(f.url())))
    }

    /// `page.addScriptTag({ content | url | path, type })`: waits until the
    /// script ran (or loaded).
    pub async fn add_script_tag(&self, tag: ScriptTag) -> Result<()> {
        self.main_frame().add_script_tag(tag).await
    }

    /// The offset of a browsing context's viewport in the page's viewport
    /// (summed over its ancestors' iframes); `None` once it is gone.
    pub(crate) async fn context_offset(&self, context: &str) -> Result<Option<(f64, f64)>> {
        let frames = self.frames().await?;
        let Some(mut frame) = frames.into_iter().find(|f| f.context == context) else {
            return Ok(None);
        };
        let mut offset = (0.0, 0.0);
        while let Some(parent) = frame.parent.clone() {
            match self.child_offset(&parent.context, &frame.context).await? {
                Some((x, y)) => offset = (offset.0 + x, offset.1 + y),
                None => return Ok(None),
            }
            frame = *parent;
        }
        Ok(Some(offset))
    }

    /// The content-box offset, in `parent`'s viewport, of the iframe whose
    /// window is the browsing context `child`.
    async fn child_offset(&self, parent: &str, child: &str) -> Result<Option<(f64, f64)>> {
        let r = self
            .script_evaluate_in(
                parent,
                "[...document.querySelectorAll('iframe, frame')].map(f => f.contentWindow)",
                Some(UTILITY),
            )
            .await?;
        let Some(index) = r
            .pointer("/result/value")
            .and_then(Value::as_array)
            .and_then(|ws| {
                ws.iter().position(|w| {
                    w.pointer("/value/context").and_then(Value::as_str) == Some(child)
                })
            })
        else {
            return Ok(None);
        };
        let v = self
            .call_json_in(
                parent,
                "(n) => { const f = document.querySelectorAll('iframe, frame')[n]; if (!f) return null; \
                 const r = f.getBoundingClientRect(); const s = getComputedStyle(f); \
                 return { x: r.left + f.clientLeft + parseFloat(s.paddingLeft), \
                          y: r.top + f.clientTop + parseFloat(s.paddingTop) }; }",
                &[json!(index)],
                Some(UTILITY),
            )
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some((
            v["x"].as_f64().unwrap_or_default(),
            v["y"].as_f64().unwrap_or_default(),
        )))
    }
}

impl Frame {
    selector_shortcuts!();

    /// `frame.page()`.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// `frame.name()`: the frame's `window.name`.
    pub async fn name(&self) -> Result<String> {
        self.evaluate("window.name").await
    }

    /// `frame.parentFrame()`.
    pub fn parent_frame(&self) -> Option<Frame> {
        self.parent.as_deref().cloned()
    }

    /// `frame.childFrames()`.
    pub async fn child_frames(&self) -> Result<Vec<Frame>> {
        Ok(self
            .page
            .frames()
            .await?
            .into_iter()
            .filter(|f| f.parent.as_ref().is_some_and(|p| p.context == self.context))
            .collect())
    }

    /// `frame.isDetached()`: the frame is no longer in the page.
    pub async fn is_detached(&self) -> Result<bool> {
        if self.page.is_closed() {
            return Ok(true);
        }
        Ok(!self
            .page
            .frames()
            .await?
            .iter()
            .any(|f| f.context == self.context))
    }

    /// `frame.goto(url)`: navigates the frame and waits for its `load`.
    pub async fn goto(&self, url: &str) -> Result<()> {
        self.page.goto_in(&self.context, url).await
    }

    /// `frame.title()`.
    pub async fn title(&self) -> Result<String> {
        self.evaluate("document.title").await
    }

    /// `frame.content()`.
    pub async fn content(&self) -> Result<String> {
        self.page.content_in(&self.context).await
    }

    /// `frame.setContent(html)`.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.page.set_content_in(&self.context, html).await
    }

    /// `frame.evaluate(expressionOrFunction)` (JSON result).
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self
            .page
            .evaluate_value_in(json!({"context": self.context}), source, None)
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
            .page
            .evaluate_value_in(json!({"context": self.context}), source, Some(&arg))
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `frame.waitForFunction(fn | expression)`: its first truthy value.
    pub async fn wait_for_function(&self, source: &str) -> Result<Value> {
        self.page.wait_for_function_in(&self.context, source).await
    }

    /// `frame.waitForTimeout(duration)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.page.sleep(duration).await
    }

    /// `frame.waitForLoadState(state)` from the frame's `readyState`
    /// (`networkidle` waits for the page's network to go quiet).
    pub async fn wait_for_load_state(&self, state: LifecycleEvent) -> Result<()> {
        if self.parent.is_none() || state == LifecycleEvent::NetworkIdle {
            return self.page.wait_for_load_state(state).await;
        }
        let ready = match state {
            LifecycleEvent::Commit => return Ok(()),
            LifecycleEvent::DomContentLoaded => "document.readyState !== 'loading'",
            _ => "document.readyState === 'complete'",
        };
        self.wait_for_function(ready).await.map(drop)
    }

    /// `frame.waitForURL(url)`: until the frame's URL matches, then `load`.
    pub async fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let matcher = url.into();
        let deadline = self.page.now() + TIMEOUT;
        loop {
            let now: String = self.evaluate("location.href").await.unwrap_or_default();
            if matcher.matches(&now) {
                break;
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for URL, now {now}"),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
        self.wait_for_load_state(LifecycleEvent::Load).await
    }

    /// `frame.addScriptTag(...)`: waits until the script ran (or loaded).
    pub async fn add_script_tag(&self, tag: ScriptTag) -> Result<()> {
        let args = tag.to_args(&*self.page.services().fs)?;
        self.page
            .call_json_in(&self.context, ADD_SCRIPT, &args, None)
            .await
            .map(drop)
    }

    /// `frame.addStyleTag(...)`.
    pub async fn add_style_tag(&self, tag: StyleTag) -> Result<()> {
        self.page.add_style_tag_in(&self.context, tag).await
    }

    /// `frame.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            root: Some(self.context.clone()),
            frames: Vec::new(),
            selector: selector.to_owned(),
        }
    }

    /// `frame.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            root: Some(self.context.clone()),
            frames: vec![selector.to_owned()],
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
