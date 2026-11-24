//! JSHandle and ElementHandle on the WebKit engine, over WebKit remote
//! objects (`Runtime.evaluate` / `Runtime.callFunctionOn` without
//! `returnByValue`). An element handle registers its node with Playwright's
//! injected script in the utility world (the `chromiumoxide-handle`
//! selector engine, as on Chromium and Firefox), so every locator action
//! works on it; `DOM.resolveNode` moves nodes between worlds.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::locator::{BoundingBox, WaitForState};
use crate::application::webkit::Page;
use crate::application::webkit_frame::Frame;
use crate::application::webkit_locator::Locator;
use crate::application::webkit_worker::Worker;
use crate::domain::error::{Error, Result};

/// Playwright's `JSHandle`: a reference to a value in a page.
#[derive(Clone, Debug)]
pub struct JsHandle {
    pub(crate) page: Page,
    /// The frame the value lives in (`None`: the main frame).
    pub(crate) frame: Option<String>,
    /// The WebKit `RemoteObject` (with an `objectId` for objects).
    pub(crate) remote: Value,
    /// The worker the value lives in, if any.
    pub(crate) worker: Option<Worker>,
}

/// Playwright's `ElementHandle`: a handle to a DOM element.
#[derive(Clone, Debug)]
pub struct ElementHandle {
    handle: JsHandle,
}

impl std::ops::Deref for ElementHandle {
    type Target = JsHandle;
    fn deref(&self) -> &JsHandle {
        &self.handle
    }
}

fn thrown(r: &Value) -> Option<Error> {
    (r["wasThrown"].as_bool() == Some(true)).then(|| {
        Error::Evaluation(
            r["result"]["description"]
                .as_str()
                .unwrap_or("exception")
                .to_owned(),
        )
    })
}

/// Where remote objects live: a page target or a worker.
#[derive(Clone, Debug)]
pub(crate) enum Via {
    Page(Page),
    Worker(Worker),
}

impl Via {
    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        match self {
            Via::Page(p) => p.target_call(method, params).await,
            Via::Worker(w) => w.call(method, params).await,
        }
    }

    /// Awaits a promise result (by value or as a remote object).
    pub(crate) async fn settle(&self, mut r: Value, by_value: bool) -> Result<Value> {
        if let Some(e) = thrown(&r) {
            return Err(e);
        }
        if r["result"]["className"].as_str() == Some("Promise")
            && let Some(id) = r["result"]["objectId"].as_str()
        {
            r = self
                .call(
                    "Runtime.awaitPromise",
                    json!({"promiseObjectId": id, "returnByValue": by_value}),
                )
                .await?;
            if let Some(e) = thrown(&r) {
                return Err(e);
            }
        }
        Ok(r["result"].take())
    }

    /// `function` called on the remote object `object` with call arguments;
    /// the result's `RemoteObject` (a value when `by_value`).
    pub(crate) async fn call_on(
        &self,
        object: &str,
        function: &str,
        arguments: Vec<Value>,
        by_value: bool,
    ) -> Result<Value> {
        let r = self
            .call(
                "Runtime.callFunctionOn",
                json!({"objectId": object, "functionDeclaration": function,
                       "arguments": arguments, "returnByValue": false,
                       "emulateUserGesture": true}),
            )
            .await?;
        // A promise is awaited (by value when asked); another object is
        // read back by value when asked.
        let v = self.settle(r, by_value).await?;
        match v["objectId"].as_str() {
            Some(id) if by_value => {
                let r = self
                    .call(
                        "Runtime.callFunctionOn",
                        json!({"objectId": id, "functionDeclaration": "function() { return this; }",
                               "returnByValue": true}),
                    )
                    .await?;
                self.settle(r, true).await
            }
            _ => Ok(v),
        }
    }
}

impl Page {
    pub(crate) async fn settle(&self, r: Value, by_value: bool) -> Result<Value> {
        Via::Page(self.clone()).settle(r, by_value).await
    }

    pub(crate) async fn call_on(
        &self,
        object: &str,
        function: &str,
        arguments: Vec<Value>,
        by_value: bool,
    ) -> Result<Value> {
        Via::Page(self.clone())
            .call_on(object, function, arguments, by_value)
            .await
    }

    /// Evaluates `source` (an expression, or a function called with `arg`)
    /// in a frame's main world; a handle to the result.
    pub(crate) async fn evaluate_handle_in(
        &self,
        frame: Option<&str>,
        source: &str,
        arg: Option<&Value>,
    ) -> Result<JsHandle> {
        let source = source.trim().trim_end_matches(';');
        let arg = arg
            .map(Value::to_string)
            .unwrap_or_else(|| "undefined".into());
        let expression = format!(
            "(async (__chromiumoxide_arg) => {{ const __v = (\n{source}\n); \
             return typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; }})({arg})"
        );
        let context = self.context_of(frame, false);
        if frame.is_some() && context.is_none() {
            return Err(Error::TargetClosed(
                "the frame has no execution context".into(),
            ));
        }
        let mut params = json!({"expression": expression, "returnByValue": false,
                                "emulateUserGesture": true});
        if let Some(id) = context {
            params["contextId"] = json!(id);
        }
        let r = self.target_call("Runtime.evaluate", params).await?;
        let remote = self.settle(r, false).await?;
        Ok(JsHandle {
            page: self.clone(),
            frame: frame.map(str::to_owned),
            remote,
            worker: None,
        })
    }

    /// `page.evaluateHandle(expressionOrFunction)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        self.evaluate_handle_in(None, source, None).await
    }

    /// `page.evaluateHandle(fn, arg)`.
    pub async fn evaluate_handle_with<A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<JsHandle> {
        let arg = serde_json::to_value(arg)?;
        self.evaluate_handle_in(None, source, Some(&arg)).await
    }

    /// `page.waitForSelector(selector, { state })`: the element once it is
    /// attached or visible; `None` for `detached` and `hidden`.
    pub async fn wait_for_selector(
        &self,
        selector: &str,
        state: WaitForState,
    ) -> Result<Option<ElementHandle>> {
        wait_for_selector(self.locator(selector), state).await
    }

    /// The node `object` moved to a frame's main or utility world.
    pub(crate) async fn adopt(
        &self,
        object: &str,
        frame: Option<&str>,
        utility: bool,
    ) -> Result<Value> {
        let context = self
            .context_of(frame, utility)
            .ok_or_else(|| Error::TargetClosed("the frame has no execution context".into()))?;
        let r = self
            .target_call(
                "DOM.resolveNode",
                json!({"objectId": object, "executionContextId": context}),
            )
            .await?;
        Ok(r["object"].clone())
    }
}

async fn wait_for_selector(locator: Locator, state: WaitForState) -> Result<Option<ElementHandle>> {
    locator.wait_for(state).await?;
    match state {
        WaitForState::Attached | WaitForState::Visible => Ok(Some(locator.element_handle().await?)),
        WaitForState::Detached | WaitForState::Hidden => Ok(None),
    }
}

impl Frame {
    /// `frame.evaluateHandle(expressionOrFunction)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        self.page
            .evaluate_handle_in(self.scope(), source, None)
            .await
    }

    /// `frame.waitForSelector(selector, { state })`.
    pub async fn wait_for_selector(
        &self,
        selector: &str,
        state: WaitForState,
    ) -> Result<Option<ElementHandle>> {
        wait_for_selector(self.locator(selector), state).await
    }

    /// `frame.frameElement()`: the `<iframe>` (or `<frame>`) element that
    /// holds this frame (`DOM.resolveNode` with the frame id).
    pub async fn frame_element(&self) -> Result<ElementHandle> {
        let parent = self
            .parent_frame()
            .ok_or_else(|| Error::InvalidArgument("Frame has been detached.".into()))?;
        let context = self
            .page
            .context_of(parent.scope(), false)
            .ok_or_else(|| Error::InvalidArgument("Frame has been detached.".into()))?;
        let r = self
            .page
            .target_call(
                "DOM.resolveNode",
                json!({"frameId": self.id, "executionContextId": context}),
            )
            .await?;
        Ok(ElementHandle {
            handle: JsHandle {
                page: self.page.clone(),
                frame: parent.scope().map(str::to_owned),
                remote: r["object"].clone(),
                worker: None,
            },
        })
    }

    /// `frame.waitForNavigation()` (deprecated in Playwright: use
    /// `wait_for_url`): the frame's next document (a new execution
    /// context), once loaded. Start it before the action.
    pub fn wait_for_navigation(&self) -> impl Future<Output = Result<()>> + Send + use<> {
        let frame = self.clone();
        let before = self.page.context_of(self.scope(), false);
        async move {
            let page = &frame.page;
            let timeout = page.navigation_timeout();
            let deadline = page.now() + timeout;
            loop {
                let now = page.context_of(frame.scope(), false);
                if now.is_some() && now != before {
                    let done: bool = frame
                        .evaluate("document.readyState === 'complete'")
                        .await
                        .unwrap_or(false);
                    if done {
                        return Ok(());
                    }
                }
                if page.now() >= deadline {
                    return Err(Error::Timeout {
                        ms: timeout.as_millis() as u64,
                        action: "waiting for navigation".into(),
                    });
                }
                page.sleep(Duration::from_millis(16)).await;
            }
        }
    }
}

impl Locator {
    /// `locator.elementHandle()`: a handle to the single matching element
    /// (waits for it).
    pub async fn element_handle(&self) -> Result<ElementHandle> {
        let (frame, object) = self.element_object().await?;
        let remote = self.page.adopt(&object, frame.as_deref(), false).await?;
        Ok(ElementHandle {
            handle: JsHandle {
                page: self.page.clone(),
                frame,
                remote,
                worker: None,
            },
        })
    }

    /// `locator.elementHandles()`: handles to every matching element now.
    pub async fn element_handles(&self) -> Result<Vec<ElementHandle>> {
        let n = self.count().await?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n as i64 {
            out.push(self.nth(i).element_handle().await?);
        }
        Ok(out)
    }

    /// `locator.evaluateHandle(fn)`: `fn(element)` as a handle.
    pub async fn evaluate_handle(&self, function: &str) -> Result<JsHandle> {
        self.element_handle().await?.evaluate_handle(function).await
    }
}

impl JsHandle {
    fn object_id(&self) -> Option<&str> {
        self.remote["objectId"].as_str()
    }

    fn via(&self) -> Via {
        match &self.worker {
            Some(w) => Via::Worker(w.clone()),
            None => Via::Page(self.page.clone()),
        }
    }

    /// The JSON value of `source` where this value lives.
    async fn value_here(&self, source: &str, arg: &Value) -> Result<Value> {
        match &self.worker {
            Some(w) => w.evaluate_value(source, Some(arg)).await,
            None => {
                self.page
                    .evaluate_value_in(source, Some(arg), self.frame.as_deref())
                    .await
            }
        }
    }

    /// A handle to the value of `source` where this value lives.
    async fn handle_here(&self, source: &str, arg: &Value) -> Result<JsHandle> {
        match &self.worker {
            Some(w) => w.evaluate_handle_value(source, Some(arg)).await,
            None => {
                self.page
                    .evaluate_handle_in(self.frame.as_deref(), source, Some(arg))
                    .await
            }
        }
    }

    /// `jsHandle.evaluate(fn)`: `fn(value)` as JSON.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        self.evaluate_with(function, Value::Null).await
    }

    /// `jsHandle.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let function = function.trim().trim_end_matches(';');
        let arg = serde_json::to_value(arg)?;
        let v = match self.object_id() {
            Some(object) => {
                let r = self
                    .via()
                    .call_on(
                        object,
                        &format!(
                            "function(__a) {{ return (async () => {{ \
                             const __r = await ({function})(this, JSON.parse(__a)); \
                             return __r === undefined ? undefined : JSON.stringify(__r); }})(); }}"
                        ),
                        vec![json!({"value": arg.to_string()})],
                        true,
                    )
                    .await?;
                match r["value"].as_str() {
                    Some(text) => serde_json::from_str(text)
                        .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}")))?,
                    None => Value::Null,
                }
            }
            None => {
                let value = self.remote.get("value").cloned().unwrap_or(Value::Null);
                self.value_here(
                    &format!("(__p) => ({function})(__p.v, __p.a)"),
                    &json!({"v": value, "a": arg}),
                )
                .await?
            }
        };
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `jsHandle.evaluateHandle(fn)`: `fn(value)` as a handle.
    pub async fn evaluate_handle(&self, function: &str) -> Result<JsHandle> {
        self.evaluate_handle_with(function, Value::Null).await
    }

    /// `jsHandle.evaluateHandle(fn, arg)`.
    pub async fn evaluate_handle_with<A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<JsHandle> {
        let function = function.trim().trim_end_matches(';');
        let arg = serde_json::to_value(arg)?;
        match self.object_id() {
            Some(object) => {
                let remote = self
                    .via()
                    .call_on(
                        object,
                        &format!(
                            "function(__a) {{ return (async () => \
                             await ({function})(this, JSON.parse(__a)))(); }}"
                        ),
                        vec![json!({"value": arg.to_string()})],
                        false,
                    )
                    .await?;
                Ok(JsHandle {
                    page: self.page.clone(),
                    frame: self.frame.clone(),
                    remote,
                    worker: self.worker.clone(),
                })
            }
            None => {
                let value = self.remote.get("value").cloned().unwrap_or(Value::Null);
                self.handle_here(
                    &format!("(__p) => ({function})(__p.v, __p.a)"),
                    &json!({"v": value, "a": arg}),
                )
                .await
            }
        }
    }

    /// `jsHandle.jsonValue()`.
    pub async fn json_value(&self) -> Result<Value> {
        if self.object_id().is_none() {
            return Ok(self.remote.get("value").cloned().unwrap_or(Value::Null));
        }
        self.evaluate("v => v").await
    }

    /// `jsHandle.getProperty(name)`.
    pub async fn get_property(&self, name: &str) -> Result<JsHandle> {
        self.evaluate_handle_with("(o, n) => o[n]", name).await
    }

    /// `jsHandle.getProperties()`: the own enumerable properties.
    pub async fn get_properties(&self) -> Result<Vec<(String, JsHandle)>> {
        let names: Vec<String> = self.evaluate("o => Object.keys(o)").await?;
        let mut out = Vec::new();
        for name in names {
            let value = self.get_property(&name).await?;
            out.push((name, value));
        }
        Ok(out)
    }

    /// `jsHandle.asElement()`: the element handle, if this is an element.
    pub fn as_element(&self) -> Option<ElementHandle> {
        (self.remote["subtype"] == "node" && self.worker.is_none()).then(|| ElementHandle {
            handle: self.clone(),
        })
    }

    /// `jsHandle.dispose()`: releases the reference.
    pub async fn dispose(&self) -> Result<()> {
        if let Some(object) = self.object_id() {
            self.via()
                .call("Runtime.releaseObject", json!({"objectId": object}))
                .await?;
        }
        Ok(())
    }
}

/// Playwright's ElementHandle methods: the element's pinned locator's.
macro_rules! element_actions {
    ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
        $(
            pub async fn $name(&self, $($arg: $ty),*) -> Result<$ret> {
                self.locator().await?.$name($($arg),*).await
            }
        )*
    };
}

impl ElementHandle {
    /// The frame the element is in (`DOM.describeNode`'s `ownerFrameId`).
    async fn owner_frame_id(&self) -> Result<Option<String>> {
        let object = self
            .object_id()
            .ok_or_else(|| Error::Evaluation("not an element".into()))?;
        let r = self
            .handle
            .page
            .target_call("DOM.describeNode", json!({"objectId": object}))
            .await?;
        Ok(r["ownerFrameId"].as_str().map(str::to_owned))
    }

    /// A locator that matches exactly this element: its node, registered
    /// with the injected script in its frame's utility world.
    async fn locator(&self) -> Result<Locator> {
        let page = &self.handle.page;
        let frame = match self.owner_frame_id().await? {
            Some(f) => Some(f),
            None => self.handle.frame.clone(),
        };
        let object = self
            .object_id()
            .ok_or_else(|| Error::Evaluation("not an element".into()))?;
        // Installs Playwright's injected script (with the handle engine).
        page.injected_call_in(frame.as_deref(), "() => 0", &[])
            .await?;
        let utility = page.adopt(object, frame.as_deref(), true).await?;
        let utility = utility["objectId"]
            .as_str()
            .ok_or_else(|| Error::Evaluation("element is not attached".into()))?;
        let id = page
            .call_on(
                utility,
                "function() { const m = globalThis.__chromiumoxide_handles || \
                 (globalThis.__chromiumoxide_handles = new Map()); \
                 const id = 'w' + (globalThis.__chromiumoxide_handle_seq = \
                 (globalThis.__chromiumoxide_handle_seq || 0) + 1); m.set(id, this); return id; }",
                vec![],
                true,
            )
            .await?;
        let _ = page
            .target_call("Runtime.releaseObject", json!({"objectId": utility}))
            .await;
        Ok(Locator {
            page: page.clone(),
            root: frame,
            frames: Vec::new(),
            selector: format!(
                "chromiumoxide-handle={}",
                id["value"].as_str().unwrap_or_default()
            ),
        })
    }

    element_actions! {
        check() -> ();
        uncheck() -> ();
        set_checked(checked: bool) -> ();
        click() -> ();
        dblclick() -> ();
        tap() -> ();
        hover() -> ();
        focus() -> ();
        fill(value: &str) -> ();
        press(key: &str) -> ();
        select_text() -> ();
        dispatch_event(kind: &str) -> ();
        scroll_into_view_if_needed() -> ();
        select_option(values: &[&str]) -> Vec<String>;
        set_input_files(files: &[PathBuf]) -> ();
        get_attribute(name: &str) -> Option<String>;
        text_content() -> Option<String>;
        inner_text() -> String;
        inner_html() -> String;
        input_value() -> String;
        is_checked() -> bool;
        is_disabled() -> bool;
        is_editable() -> bool;
        is_enabled() -> bool;
        is_hidden() -> bool;
        is_visible() -> bool;
        bounding_box() -> Option<BoundingBox>;
        screenshot() -> Vec<u8>;
    }

    /// `elementHandle.type(text)` (deprecated in Playwright: use
    /// `press_sequentially`).
    #[deprecated(note = "use locator.press_sequentially, like Playwright")]
    pub async fn type_text(&self, text: &str) -> Result<()> {
        self.locator().await?.press_sequentially(text).await
    }

    /// `elementHandle.waitForElementState(state)`: `visible`, `hidden`,
    /// `stable`, `enabled`, `disabled` or `editable`.
    pub async fn wait_for_element_state(&self, state: &str) -> Result<()> {
        let locator = self.locator().await?;
        let page = &self.handle.page;
        let deadline = page.now() + page.timeout();
        loop {
            let v = locator
                .read(
                    "async (i, node, state) => { const s = await i.checkElementStates(node, [state], true); \
                     if (s === 'error:notconnected') throw i.createStacklessError('Element is not attached to the DOM'); \
                     return !s; }",
                    &[json!(state)],
                )
                .await?;
            if v == Value::Bool(true) {
                return Ok(());
            }
            if page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: page.timeout().as_millis() as u64,
                    action: format!("waiting for element to be {state}"),
                });
            }
            page.sleep(Duration::from_millis(16)).await;
        }
    }

    /// `elementHandle.waitForSelector(selector, { state })`: below this
    /// element.
    pub async fn wait_for_selector(
        &self,
        selector: &str,
        state: WaitForState,
    ) -> Result<Option<ElementHandle>> {
        wait_for_selector(self.locator().await?.locator(selector), state).await
    }

    /// `elementHandle.contentFrame()`: the frame of an `<iframe>`.
    pub async fn content_frame(&self) -> Result<Option<Frame>> {
        let Some(object) = self.object_id() else {
            return Ok(None);
        };
        let r = self
            .handle
            .page
            .target_call("DOM.describeNode", json!({"objectId": object}))
            .await?;
        let Some(id) = r["contentFrameId"].as_str() else {
            return Ok(None);
        };
        Ok(self
            .handle
            .page
            .frames()
            .await?
            .into_iter()
            .find(|f| f.id == id))
    }

    /// `elementHandle.ownerFrame()`: the frame the element is in.
    pub async fn owner_frame(&self) -> Result<Option<Frame>> {
        let Some(id) = self.owner_frame_id().await? else {
            return Ok(None);
        };
        Ok(self
            .handle
            .page
            .frames()
            .await?
            .into_iter()
            .find(|f| f.id == id))
    }
}
