//! Frames on the WebKit engine: `page.frameLocator()` (locators that run in
//! an iframe's document) and `page.frames()`. An iframe element's frame id
//! comes from `DOM.describeNode` (`contentFrameId`), as in Playwright's
//! `WKPage.getContentFrame`.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::actions::StyleTag;
use crate::application::firefox_frame::selector_shortcuts;
use crate::application::scripts::{ADD_SCRIPT, ScriptTag};
use crate::application::webkit::Page;
use crate::application::webkit_locator::Locator;
use crate::domain::error::{Error, Result};
use crate::domain::frame::LifecycleEvent;
use crate::domain::selectors::{self, RoleOptions, TextMatch};
use crate::domain::url_matcher::UrlMatcher;

/// How long a found iframe may take to report its execution contexts.
const FRAME_READY: Duration = Duration::from_secs(5);

/// `page.frameLocator(selector)`: locators inside an iframe.
#[derive(Clone, Debug)]
pub struct FrameLocator {
    pub(crate) page: Page,
    pub(crate) root: Option<String>,
    pub(crate) frames: Vec<String>,
}

impl FrameLocator {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            root: self.root.clone(),
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
            root: self.root.clone(),
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
    pub(crate) page: Page,
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) main: bool,
    /// The parent frame's id (`None` for the main frame).
    pub(crate) parent: Option<String>,
    /// The frame's ancestors, the main frame first.
    pub(crate) ancestors: Vec<Frame>,
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
    /// The frame id to run in (`None`: the main frame).
    pub(crate) fn scope(&self) -> Option<&str> {
        (!self.main).then_some(self.id.as_str())
    }
    /// `frame.evaluate(expression or function)` in the frame's main world.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self
            .page
            .evaluate_value_in(source, None, self.scope())
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
            .evaluate_value_in(source, Some(&arg), self.scope())
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

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
        self.ancestors.last().cloned()
    }

    /// `frame.childFrames()`.
    pub async fn child_frames(&self) -> Result<Vec<Frame>> {
        Ok(self
            .page
            .frames()
            .await?
            .into_iter()
            .filter(|f| f.parent.as_deref() == Some(self.id.as_str()))
            .collect())
    }

    /// `frame.isDetached()`: the frame is no longer in the page.
    pub async fn is_detached(&self) -> Result<bool> {
        if self.page.is_closed() {
            return Ok(true);
        }
        Ok(!self.page.frames().await?.iter().any(|f| f.id == self.id))
    }

    /// `frame.goto(url)`: navigates the frame and waits for its `load`.
    pub async fn goto(&self, url: &str) -> Result<()> {
        if self.main {
            return self.page.goto(url).await;
        }
        let before = self.page.context_of(Some(&self.id), false);
        self.page
            .inner
            .wk
            .call(
                None,
                "Playwright.navigate",
                json!({"url": url, "pageProxyId": self.page.inner.proxy, "frameId": self.id}),
            )
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        // The new document has a new execution context; then its `load`.
        let timeout = self.page.navigation_timeout();
        let deadline = self.page.now() + timeout;
        loop {
            let now = self.page.context_of(Some(&self.id), false);
            if now.is_some() && now != before {
                let done: bool = self
                    .evaluate("document.readyState === 'complete'")
                    .await
                    .unwrap_or(false);
                if done {
                    return Ok(());
                }
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: timeout.as_millis() as u64,
                    action: format!("navigating frame to {url}"),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `frame.title()`.
    pub async fn title(&self) -> Result<String> {
        self.evaluate("document.title").await
    }

    /// `frame.content()`.
    pub async fn content(&self) -> Result<String> {
        self.page.content_in(self.scope()).await
    }

    /// `frame.setContent(html)`.
    pub async fn set_content(&self, html: &str) -> Result<()> {
        self.page.set_content_in(self.scope(), html).await
    }

    /// `frame.waitForFunction(fn | expression)`: its first truthy value.
    pub async fn wait_for_function(&self, source: &str) -> Result<Value> {
        self.page.wait_for_function_in(self.scope(), source).await
    }

    /// `frame.waitForTimeout(duration)`.
    pub async fn wait_for_timeout(&self, duration: Duration) {
        self.page.sleep(duration).await
    }

    /// `frame.waitForLoadState(state)` from the frame's `readyState`
    /// (`networkidle`: `load`, then no new resource for 500 ms, from the
    /// Resource Timing entries).
    pub async fn wait_for_load_state(&self, state: LifecycleEvent) -> Result<()> {
        let ready = match state {
            LifecycleEvent::Commit => return Ok(()),
            LifecycleEvent::DomContentLoaded => "document.readyState !== 'loading'",
            LifecycleEvent::Load => "document.readyState === 'complete'",
            LifecycleEvent::NetworkIdle => {
                "new Promise(r => { if (document.readyState !== 'complete') return r(false); \
                 const n = performance.getEntriesByType('resource').length; \
                 setTimeout(() => r(performance.getEntriesByType('resource').length === n), 500); })"
            }
        };
        self.wait_for_function(ready).await.map(drop)
    }

    /// `frame.waitForURL(url)`: until the frame's URL matches, then `load`.
    pub async fn wait_for_url(&self, url: impl Into<UrlMatcher>) -> Result<()> {
        let matcher = url.into();
        let timeout = self.page.navigation_timeout();
        let deadline = self.page.now() + timeout;
        loop {
            let now: String = self.evaluate("location.href").await.unwrap_or_default();
            if matcher.matches(&now) {
                break;
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: timeout.as_millis() as u64,
                    action: format!("waiting for URL, now {now}"),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
        self.wait_for_load_state(LifecycleEvent::Load).await
    }

    /// `frame.addScriptTag(...)`: waits until the script ran (or loaded).
    pub async fn add_script_tag(&self, tag: ScriptTag) -> Result<()> {
        let args = tag.to_args(&*self.page.inner.services.fs)?;
        self.page
            .call_json_frame(ADD_SCRIPT, &args, self.scope(), false)
            .await
            .map(drop)
    }

    /// `frame.addStyleTag(...)`.
    pub async fn add_style_tag(&self, tag: StyleTag) -> Result<()> {
        self.page.add_style_tag_in(self.scope(), tag).await
    }

    /// `frame.locator(selector)`.
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            root: self.scope().map(str::to_owned),
            frames: Vec::new(),
            selector: selector.to_owned(),
        }
    }

    /// `frame.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.page.clone(),
            root: self.scope().map(str::to_owned),
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

impl Page {
    /// `page.frameLocator(selector)`.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.clone(),
            root: None,
            frames: vec![selector.to_owned()],
        }
    }

    /// `page.frames()`: the main frame first, then its descendants in
    /// document order (`Page.getResourceTree`).
    pub async fn frames(&self) -> Result<Vec<Frame>> {
        let tree = self.target_call("Page.getResourceTree", json!({})).await?;
        let mut out = Vec::new();
        fn walk(page: &Page, node: &Value, ancestors: &[Frame], out: &mut Vec<Frame>) {
            let f = &node["frame"];
            let frame = Frame {
                page: page.clone(),
                id: f["id"].as_str().unwrap_or_default().to_owned(),
                url: f["url"].as_str().unwrap_or_default().to_owned(),
                main: ancestors.is_empty(),
                parent: ancestors.last().map(|p| p.id.clone()),
                ancestors: ancestors.to_vec(),
            };
            out.push(frame.clone());
            let mut chain = ancestors.to_vec();
            chain.push(frame);
            for child in node["childFrames"].as_array().into_iter().flatten() {
                walk(page, child, &chain, out);
            }
        }
        walk(self, &tree["frameTree"], &[], &mut out);
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

impl Page {
    selector_shortcuts!();

    /// `page.mainFrame()`.
    pub fn main_frame(&self) -> Frame {
        Frame {
            page: self.clone(),
            id: self.main_frame_id(),
            url: self.url(),
            main: true,
            parent: None,
            ancestors: Vec::new(),
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

    /// `page.addScriptTag({ content | url | path, type })`.
    pub async fn add_script_tag(&self, tag: ScriptTag) -> Result<()> {
        self.main_frame().add_script_tag(tag).await
    }

    /// The offset of a frame's viewport in the page's viewport (summed over
    /// its ancestors' iframes); `None` once it is gone.
    pub(crate) async fn frame_offset(&self, frame: &str) -> Result<Option<(f64, f64)>> {
        let Some(f) = self.frames().await?.into_iter().find(|f| f.id == frame) else {
            return Ok(None);
        };
        if f.main {
            return Ok(Some((0.0, 0.0)));
        }
        let mut chain: Vec<&Frame> = f.ancestors.iter().collect();
        chain.push(&f);
        let mut offset = (0.0, 0.0);
        for pair in chain.windows(2) {
            match self.child_offset(pair[0].scope(), &pair[1].id).await? {
                Some((x, y)) => offset = (offset.0 + x, offset.1 + y),
                None => return Ok(None),
            }
        }
        Ok(Some(offset))
    }

    /// The content-box offset, in `parent`'s viewport, of the iframe whose
    /// frame is `child`.
    async fn child_offset(&self, parent: Option<&str>, child: &str) -> Result<Option<(f64, f64)>> {
        let count = self
            .call_json_frame(
                "() => document.querySelectorAll('iframe, frame').length",
                &[],
                parent,
                true,
            )
            .await?
            .as_u64()
            .unwrap_or(0);
        for n in 0..count {
            let element = self
                .run_in(
                    &format!("document.querySelectorAll('iframe, frame')[{n}]"),
                    parent,
                    true,
                )
                .await?;
            let Some(object) = element["objectId"].as_str() else {
                continue;
            };
            let node = self
                .target_call("DOM.describeNode", json!({"objectId": object}))
                .await?;
            if node["contentFrameId"].as_str() != Some(child) {
                continue;
            }
            let v = self
                .call_json_frame(
                    "(n) => { const f = document.querySelectorAll('iframe, frame')[n]; if (!f) return null; \
                     const r = f.getBoundingClientRect(); const s = getComputedStyle(f); \
                     return { x: r.left + f.clientLeft + parseFloat(s.paddingLeft), \
                              y: r.top + f.clientTop + parseFloat(s.paddingTop) }; }",
                    &[json!(n)],
                    parent,
                    true,
                )
                .await?;
            if v.is_null() {
                return Ok(None);
            }
            return Ok(Some((
                v["x"].as_f64().unwrap_or_default(),
                v["y"].as_f64().unwrap_or_default(),
            )));
        }
        Ok(None)
    }
}
