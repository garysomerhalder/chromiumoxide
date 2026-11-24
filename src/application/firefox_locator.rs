//! Locators on the Firefox engine: Playwright's injected script in a BiDi
//! sandbox (Firefox's equivalent of Chromium's utility world).

use std::time::Duration;

use serde_json::{Value, json};

use crate::application::firefox::{Page, UTILITY};
use crate::domain::error::{Error, Result};
use crate::domain::firefox::webdriver_key;
use crate::domain::input::{resolve_smart_modifier, split_shortcut};
use crate::domain::selectors::{self, RoleOptions, TextMatch, compose};

/// `page.locator(selector)` and the `get_by_*` builders on a Firefox page.
impl Page {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.clone(),
            frames: Vec::new(),
            selector: selector.to_owned(),
        }
    }

    /// `page.frameLocator(selector)`: locators inside that iframe.
    pub fn frame_locator(&self, selector: &str) -> FrameLocator {
        FrameLocator {
            page: self.clone(),
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

/// A Firefox locator: a selector resolved lazily, strict like Playwright's.
#[derive(Clone, Debug)]
pub struct Locator {
    pub(crate) page: Page,
    /// Iframe selectors from the main frame down to the locator's frame.
    pub(crate) frames: Vec<String>,
    pub(crate) selector: String,
}

/// `page.frameLocator(selector)`: locators inside an iframe.
#[derive(Clone, Debug)]
pub struct FrameLocator {
    pub(crate) page: Page,
    pub(crate) frames: Vec<String>,
}

impl FrameLocator {
    pub fn locator(&self, selector: &str) -> Locator {
        Locator {
            page: self.page.clone(),
            frames: self.frames.clone(),
            selector: selector.to_owned(),
        }
    }
    /// `frameLocator.frameLocator(selector)`: an iframe inside this one.
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
    pub fn get_by_label(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_label(&text.into(), exact))
    }
    pub fn get_by_placeholder(&self, text: impl Into<TextMatch>, exact: bool) -> Locator {
        self.locator(&selectors::get_by_placeholder(&text.into(), exact))
    }
    pub fn get_by_test_id(&self, id: impl Into<TextMatch>) -> Locator {
        self.locator(&selectors::get_by_test_id("data-testid", &id.into()))
    }
}

/// Default action timeout (Playwright's 30 s).
const TIMEOUT: Duration = Duration::from_secs(30);

/// Records the drag session the next pointer actions start: Firefox's
/// WebDriver actions run `dragstart`/`dragover` but end the session on
/// release without a `drop` (a TODO in its `EventUtils.js`).
const DRAG_WATCH: &str = "() => {
  globalThis.__chromiumoxide_drag?.stop();
  const w = { started: false, source: null, data: [], accepted: false, dropped: false, ended: false };
  const on = {
    dragstart: e => {
      if (e.defaultPrevented) return;
      w.started = true; w.source = e.target;
      w.data = [...e.dataTransfer.types].map(t => [t, e.dataTransfer.getData(t)]);
    },
    dragover: e => { w.accepted = e.defaultPrevented; },
    drop: () => { w.dropped = true; },
    dragend: () => { w.ended = true; },
  };
  for (const [t, f] of Object.entries(on)) window.addEventListener(t, f);
  w.stop = () => { for (const [t, f] of Object.entries(on)) window.removeEventListener(t, f); };
  globalThis.__chromiumoxide_drag = w;
  return true;
}";

/// Finishes a recorded drag the way a browser does: `drop` on the element
/// under the pointer when its last `dragover` accepted it, then `dragend`.
const DRAG_FINISH: &str = "(x, y) => {
  const w = globalThis.__chromiumoxide_drag;
  delete globalThis.__chromiumoxide_drag;
  w?.stop();
  if (!w || !w.started || w.dropped) return false;
  const dt = new DataTransfer();
  for (const [t, v] of w.data) dt.setData(t, v);
  const init = { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, dataTransfer: dt };
  const target = document.elementFromPoint(x, y);
  if (w.accepted && target) target.dispatchEvent(new DragEvent('drop', init));
  if (!w.ended && w.source) w.source.dispatchEvent(new DragEvent('dragend', init));
  return true;
}";

impl Locator {
    pub(crate) fn with(&self, selector: String) -> Locator {
        Locator {
            page: self.page.clone(),
            frames: self.frames.clone(),
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

    /// The locator's frame: its browsing context and the offset of its
    /// viewport in the page's; `None` while an iframe on the way is missing.
    pub(crate) async fn scope(&self) -> Result<Option<(String, (f64, f64))>> {
        let mut context = self.page.main_context();
        let mut offset = (0.0, 0.0);
        for frame in &self.frames {
            match self.page.child_frame(&context, frame).await? {
                Some((child, (x, y))) => {
                    context = child;
                    offset = (offset.0 + x, offset.1 + y);
                }
                None => return Ok(None),
            }
        }
        Ok(Some((context, offset)))
    }

    /// `body(injected, ...args)` in the locator's frame; while the frame is
    /// missing, `{ missing: true }` (callers treat it as no element).
    pub(crate) async fn call(&self, body: &str, args: &[Value]) -> Result<Value> {
        match self.scope().await? {
            Some((context, _)) => self.page.injected_call_in(&context, body, args).await,
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

    /// Runs `body(injected, element)` on the single element the locator
    /// resolves to, waiting until there is one (strict: more than one is an
    /// error).
    pub(crate) async fn read(&self, body: &str, args: &[Value]) -> Result<Value> {
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
                .call(&function, &[json!(self.selector), json!(args)])
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

    /// Like [`read`](Self::read), awaiting `body`'s result (which may be a
    /// promise).
    pub(crate) async fn read_value(&self, body: &str, args: &[Value]) -> Result<Value> {
        let function = format!(
            "async (i, selector, args) => {{ const parsed = i.parseSelector(selector); \
             const els = i.querySelectorAll(parsed, document); \
             if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
             if (!els.length) return {{ found: false }}; \
             const value = await ({body})(i, els[0], ...args); \
             return {{ found: true, value: value === undefined ? null : value }}; }}"
        );
        let deadline = self.page.now() + TIMEOUT;
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
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for locator('{}')", self.selector),
                });
            }
            self.page.sleep(delay).await;
            delay = (delay * 2).min(Duration::from_millis(100));
        }
    }

    /// The locator's frame content offset in the page's viewport.
    pub(crate) async fn frame_offset(&self) -> Result<(f64, f64)> {
        Ok(self.scope().await?.map(|(_, o)| o).unwrap_or_default())
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
        let deadline = self.page.now() + TIMEOUT;
        let mut delay = Duration::from_millis(20);
        loop {
            // The point is in the locator's frame; add the frames' offsets
            // for the page's viewport.
            let (r, offset) = match self.scope().await? {
                Some((context, offset)) => (
                    self.page
                        .injected_call_in(&context, PREPARE, &[json!(self.selector), json!(states)])
                        .await?,
                    offset,
                ),
                None => (json!({"retry": "waiting for the frame"}), (0.0, 0.0)),
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
        self.page
            .traced(
                "Frame",
                "click",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.click_untraced(),
            )
            .await
    }

    async fn click_untraced(&self) -> Result<()> {
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

    /// The selector as Playwright's traces show it (frames entered with
    /// `internal:control=enter-frame`).
    fn trace_selector(&self) -> String {
        self.frames
            .iter()
            .map(|f| format!("{f} >> internal:control=enter-frame"))
            .chain(std::iter::once(self.selector.clone()))
            .collect::<Vec<_>>()
            .join(" >> ")
    }

    /// `locator.dblclick()`.
    pub async fn dblclick(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "dblclick",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.dblclick_untraced(),
            )
            .await
    }

    async fn dblclick_untraced(&self) -> Result<()> {
        let (x, y) = self
            .actionable_point(&["visible", "enabled", "stable"])
            .await?;
        self.page.mouse().dblclick(x, y).await
    }

    /// `locator.dragTo(target)`: press on this element's centre, move to the
    /// target's centre (in steps, so drag and drop starts) and release.
    pub async fn drag_to(&self, target: &Locator) -> Result<()> {
        self.page.traced(
            "Frame",
            "dragAndDrop",
            serde_json::json!({"source": self.trace_selector(), "target": target.trace_selector(), "strict": true}),
            self.drag_to_untraced(target),
        )
        .await
    }

    async fn drag_to_untraced(&self, target: &Locator) -> Result<()> {
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        let (tx, ty) = target.actionable_point(&["visible", "stable"]).await?;
        let Some((context, offset)) = self.scope().await? else {
            return Err(Error::Evaluation("the frame went away".into()));
        };
        self.page
            .call_json_in(&context, DRAG_WATCH, &[], Some(UTILITY))
            .await?;
        self.page.mouse().drag(x, y, tx, ty).await?;
        self.page
            .call_json_in(
                &context,
                DRAG_FINISH,
                &[json!(tx - offset.0), json!(ty - offset.1)],
                Some(UTILITY),
            )
            .await
            .map(drop)
    }

    /// `locator.selectOption(values)`: each value matches an option's value
    /// or label; resolves with the selected values.
    pub async fn select_option(&self, values: &[&str]) -> Result<Vec<String>> {
        self.page.traced(
            "Frame",
            "selectOption",
            serde_json::json!({"selector": self.trace_selector(), "strict": true, "options": values}),
            self.select_option_untraced(values),
        )
        .await
    }

    async fn select_option_untraced(&self, values: &[&str]) -> Result<Vec<String>> {
        let options: Vec<Value> = values.iter().map(|v| json!({"valueOrLabel": v})).collect();
        let deadline = self.page.now() + TIMEOUT;
        loop {
            let r = self
                .read(
                    "(i, node, options) => i.selectOptions(node, options)",
                    &[json!(options)],
                )
                .await?;
            if let Some(values) = r.as_array() {
                return Ok(values
                    .iter()
                    .map(|v| v.as_str().unwrap_or_default().to_owned())
                    .collect());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!(
                        "selecting options in locator('{}'): {}",
                        self.selector,
                        r.as_str().unwrap_or_default()
                    ),
                });
            }
            self.page.sleep(Duration::from_millis(50)).await;
        }
    }

    /// `locator.setInputFiles(files)` (BiDi `input.setFiles`).
    pub async fn set_input_files(&self, files: &[std::path::PathBuf]) -> Result<()> {
        self.page.traced(
            "Frame",
            "setInputFiles",
            serde_json::json!({"selector": self.trace_selector(), "strict": true, "localPaths": files}),
            self.set_input_files_untraced(files),
        )
        .await
    }

    async fn set_input_files_untraced(&self, files: &[std::path::PathBuf]) -> Result<()> {
        // Waits for the element first.
        self.read("(i, node) => node.nodeName", &[]).await?;
        let Some((context, _)) = self.scope().await? else {
            return Err(Error::Evaluation("the frame went away".into()));
        };
        let node = self
            .page
            .shared_node_in(&context, &self.selector)
            .await?
            .ok_or_else(|| {
                Error::Evaluation(format!("no element for locator('{}')", self.selector))
            })?;
        let files: Vec<String> = files
            .iter()
            .map(|f| {
                std::path::absolute(f)
                    .unwrap_or_else(|_| f.clone())
                    .display()
                    .to_string()
            })
            .collect();
        self.page
            .bidi_call(
                "input.setFiles",
                json!({"context": context, "element": {"sharedId": node}, "files": files}),
            )
            .await
            .map(drop)
    }

    /// `locator.hover()`.
    pub async fn hover(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "hover",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.hover_untraced(),
            )
            .await
    }

    async fn hover_untraced(&self) -> Result<()> {
        let (x, y) = self.actionable_point(&["visible", "stable"]).await?;
        self.page
            .perform_actions(Self::pointer(vec![json!({
                "type": "pointerMove", "x": x.round() as i64, "y": y.round() as i64
            })]))
            .await
    }

    /// `locator.focus()`.
    pub async fn focus(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "focus",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.focus_untraced(),
            )
            .await
    }

    async fn focus_untraced(&self) -> Result<()> {
        self.read("(i, node) => { i.focusNode(node); }", &[])
            .await
            .map(drop)
    }

    /// `locator.fill(value)`: Playwright's injected `fill`, then the text
    /// typed in (or the selection deleted for an empty value).
    pub async fn fill(&self, value: &str) -> Result<()> {
        self.page.traced(
            "Frame",
            "fill",
            serde_json::json!({"selector": self.trace_selector(), "strict": true, "value": value}),
            self.fill_untraced(value),
        )
        .await
    }

    async fn fill_untraced(&self, value: &str) -> Result<()> {
        let r = self
            .read("(i, node, value) => i.fill(node, value)", &[json!(value)])
            .await?;
        match r.as_str() {
            Some("done") => Ok(()),
            Some("needsinput") => {
                if value.is_empty() {
                    self.press_keys("Delete").await
                } else {
                    let context = match self.scope().await? {
                        Some((context, _)) => context,
                        None => self.page.main_context(),
                    };
                    self.page.insert_text_in(&context, value).await
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
        self.page
            .traced(
                "Frame",
                "press",
                serde_json::json!({"selector": self.trace_selector(), "strict": true, "key": key}),
                self.press_untraced(key),
            )
            .await
    }

    async fn press_untraced(&self, key: &str) -> Result<()> {
        self.focus().await?;
        self.press_keys(key).await
    }

    /// `locator.pressSequentially(text)`: focuses the element and types
    /// `text` key by key.
    pub async fn press_sequentially(&self, text: &str) -> Result<()> {
        self.page.traced(
            "Frame",
            "type",
            serde_json::json!({"selector": self.trace_selector(), "strict": true, "text": text}),
            self.press_sequentially_untraced(text),
        )
        .await
    }

    async fn press_sequentially_untraced(&self, text: &str) -> Result<()> {
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
        self.page
            .traced(
                "Frame",
                "check",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.check_untraced(),
            )
            .await
    }

    async fn check_untraced(&self) -> Result<()> {
        self.set_checked(true).await
    }

    /// `locator.uncheck()`.
    pub async fn uncheck(&self) -> Result<()> {
        self.page
            .traced(
                "Frame",
                "uncheck",
                serde_json::json!({"selector": self.trace_selector(), "strict": true}),
                self.uncheck_untraced(),
            )
            .await
    }

    async fn uncheck_untraced(&self) -> Result<()> {
        self.set_checked(false).await
    }

    /// `locator.waitFor({ state })`: waits until the element is attached,
    /// visible (default), detached or hidden (strict: more than one match
    /// is an error).
    pub async fn wait_for(&self, state: crate::application::locator::WaitForState) -> Result<()> {
        use crate::application::locator::WaitForState;
        let wanted = match state {
            WaitForState::Attached => "attached",
            WaitForState::Detached => "detached",
            WaitForState::Visible => "visible",
            WaitForState::Hidden => "hidden",
        };
        let deadline = self.page.now() + TIMEOUT;
        loop {
            let v = self
                .call(
                    "(i, selector, wanted) => { const parsed = i.parseSelector(selector); \
                     const els = i.querySelectorAll(parsed, document); \
                     if (els.length > 1) throw i.strictModeViolationError(parsed, els); \
                     const el = els[0]; \
                     const visible = !!el && i.elementState(el, 'visible', true).matches; \
                     if (wanted === 'attached') return !!el; \
                     if (wanted === 'detached') return !el; \
                     if (wanted === 'visible') return visible; \
                     return !visible; }",
                    &[json!(self.selector), json!(wanted)],
                )
                .await?;
            if v == Value::Bool(true) {
                return Ok(());
            }
            if self.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: TIMEOUT.as_millis() as u64,
                    action: format!("waiting for locator('{}') to be {wanted}", self.selector),
                });
            }
            self.page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `locator.isVisible()` (does not wait).
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

    /// `locator.isHidden()` (does not wait).
    pub async fn is_hidden(&self) -> Result<bool> {
        Ok(!self.is_visible().await?)
    }

    /// `locator.count()` (does not wait).
    pub async fn count(&self) -> Result<usize> {
        let v = self
            .call(
                "(i, selector) => i.querySelectorAll(i.parseSelector(selector), document).length",
                &[json!(self.selector)],
            )
            .await?;
        Ok(v.as_u64().unwrap_or_default() as usize)
    }

    async fn all_strings(&self, map: &str) -> Result<Vec<String>> {
        let v = self
            .call(
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
