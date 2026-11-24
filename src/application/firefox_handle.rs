//! JSHandle and ElementHandle on the Firefox engine, over BiDi remote
//! references (`resultOwnership: "root"` handles, and `sharedId`s for DOM
//! nodes). An element handle registers its node with Playwright's injected
//! script (the `chromiumoxide-handle` selector engine, as on Chromium), so
//! every locator action works on it.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::firefox::{Frame, Page, UTILITY, exception_text};
use crate::application::firefox_locator::Locator;
use crate::application::locator::{BoundingBox, WaitForState};
use crate::domain::error::{Error, Result};
use crate::domain::firefox::remote_value_to_json;

/// Playwright's `JSHandle`: a reference to a value in a page or worker.
#[derive(Clone, Debug)]
pub struct JsHandle {
    pub(crate) page: Page,
    /// The BiDi script target the value lives in (`{context}` or `{realm}`).
    pub(crate) target: Value,
    /// The BiDi `RemoteValue` (with a `handle` for objects, a `sharedId`
    /// for nodes).
    pub(crate) remote: Value,
}

/// Playwright's `ElementHandle`: a handle to a DOM element.
#[derive(Clone, Debug)]
pub struct ElementHandle {
    handle: JsHandle,
    /// The browsing context of the element's document.
    context: String,
    /// The element's id in the frame's `chromiumoxide-handle` registry.
    id: String,
}

impl std::ops::Deref for ElementHandle {
    type Target = JsHandle;
    fn deref(&self) -> &JsHandle {
        &self.handle
    }
}

/// Wraps `function` so its result comes back as JSON text (or, for a
/// handle, as the value itself).
fn call_source(function: &str, as_json: bool) -> String {
    let function = function.trim().trim_end_matches(';');
    if as_json {
        format!(
            "async function(__h, __a) {{ const __r = await ({function})(__h, JSON.parse(__a)); \
             return __r === undefined ? undefined : JSON.stringify(__r); }}"
        )
    } else {
        format!("async function(__h, __a) {{ return await ({function})(__h, JSON.parse(__a)); }}")
    }
}

impl Page {
    /// `script.callFunction` with BiDi arguments in `target`; the raw result.
    pub(crate) async fn call_function_raw(
        &self,
        target: Value,
        function: &str,
        arguments: Vec<Value>,
        ownership: &str,
    ) -> Result<Value> {
        let r = self
            .bidi_call(
                "script.callFunction",
                json!({
                    "functionDeclaration": function,
                    "target": target,
                    "arguments": arguments,
                    "awaitPromise": true,
                    "resultOwnership": ownership,
                    "userActivation": true,
                }),
            )
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        Ok(r["result"].clone())
    }

    /// Evaluates `source` (an expression, or a function called with `arg`)
    /// in `target` and returns a handle to the result.
    pub(crate) async fn evaluate_handle_in(
        &self,
        target: Value,
        source: &str,
        arg: Option<&Value>,
    ) -> Result<JsHandle> {
        let source = source.trim().trim_end_matches(';');
        let arg = arg
            .map(Value::to_string)
            .unwrap_or_else(|| "undefined".into());
        let wrapped = format!(
            "(async (__chromiumoxide_arg) => {{ const __v = (\n{source}\n); \
             return typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; }})({arg})"
        );
        let r = self
            .bidi_call(
                "script.evaluate",
                json!({"expression": wrapped, "target": target, "awaitPromise": true,
                       "resultOwnership": "root", "userActivation": true}),
            )
            .await?;
        if r.get("type").and_then(Value::as_str) == Some("exception") {
            return Err(Error::Evaluation(exception_text(&r)));
        }
        Ok(JsHandle {
            page: self.clone(),
            target,
            remote: r["result"].clone(),
        })
    }

    /// `page.evaluateHandle(expressionOrFunction)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        self.evaluate_handle_in(json!({"context": self.main_context()}), source, None)
            .await
    }

    /// `page.evaluateHandle(fn, arg)`.
    pub async fn evaluate_handle_with<A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<JsHandle> {
        let arg = serde_json::to_value(arg)?;
        self.evaluate_handle_in(json!({"context": self.main_context()}), source, Some(&arg))
            .await
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

    /// Registers the node `shared_id` in `context`'s handle registry.
    pub(crate) async fn element_handle_of(
        &self,
        context: &str,
        remote: Value,
    ) -> Result<ElementHandle> {
        let shared = remote["sharedId"].as_str().unwrap_or_default().to_owned();
        // Installs Playwright's injected script (with the handle engine).
        self.injected_call_in(context, "() => 0", &[]).await?;
        let id = self
            .call_function_raw(
                json!({"context": context, "sandbox": UTILITY}),
                "(n) => { const m = globalThis.__chromiumoxide_handles || \
                 (globalThis.__chromiumoxide_handles = new Map()); \
                 const id = 'f' + (globalThis.__chromiumoxide_handle_seq = \
                 (globalThis.__chromiumoxide_handle_seq || 0) + 1); m.set(id, n); return id; }",
                vec![json!({"sharedId": shared})],
                "none",
            )
            .await?;
        Ok(ElementHandle {
            handle: JsHandle {
                page: self.clone(),
                target: json!({"context": context}),
                remote,
            },
            context: context.to_owned(),
            id: id["value"].as_str().unwrap_or_default().to_owned(),
        })
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
            .evaluate_handle_in(json!({"context": self.context}), source, None)
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
    /// holds this frame.
    pub async fn frame_element(&self) -> Result<ElementHandle> {
        let parent = self
            .parent
            .as_deref()
            .ok_or_else(|| Error::InvalidArgument("Frame has been detached.".into()))?;
        let r = self
            .page
            .call_function_raw(
                json!({"context": parent.context, "sandbox": UTILITY}),
                "() => [...document.querySelectorAll('iframe, frame')]",
                vec![],
                "none",
            )
            .await?;
        let elements = r["value"].as_array().cloned().unwrap_or_default();
        let windows = self
            .page
            .call_function_raw(
                json!({"context": parent.context, "sandbox": UTILITY}),
                "() => [...document.querySelectorAll('iframe, frame')].map(f => f.contentWindow)",
                vec![],
                "none",
            )
            .await?;
        let index = windows["value"]
            .as_array()
            .and_then(|ws| {
                ws.iter().position(|w| {
                    w.pointer("/value/context").and_then(Value::as_str)
                        == Some(self.context.as_str())
                })
            })
            .ok_or_else(|| Error::InvalidArgument("Frame has been detached.".into()))?;
        let node = elements
            .get(index)
            .cloned()
            .ok_or_else(|| Error::InvalidArgument("Frame has been detached.".into()))?;
        self.page.element_handle_of(&parent.context, node).await
    }
}

impl Locator {
    /// `locator.elementHandle()`: a handle to the single matching element
    /// (waits for it).
    pub async fn element_handle(&self) -> Result<ElementHandle> {
        self.wait_for(WaitForState::Attached).await?;
        let (context, _) = self
            .scope()
            .await?
            .ok_or_else(|| Error::Evaluation("frame of the locator is not attached".into()))?;
        let shared = self
            .page
            .shared_node_in(&context, &self.selector)
            .await?
            .ok_or_else(|| {
                Error::Evaluation(format!("no element for locator('{}')", self.selector))
            })?;
        self.page
            .element_handle_of(&context, json!({"type": "node", "sharedId": shared}))
            .await
    }

    /// `locator.elementHandles()`: handles to every matching element now.
    pub async fn element_handles(&self) -> Result<Vec<ElementHandle>> {
        let Some((context, _)) = self.scope().await? else {
            return Ok(Vec::new());
        };
        self.page.injected_call_in(&context, "() => 0", &[]).await?;
        let r = self
            .page
            .call_function_raw(
                json!({"context": context, "sandbox": UTILITY}),
                "(selector) => { const i = globalThis.__chromiumoxide_injected; \
                 return i.querySelectorAll(i.parseSelector(selector), document); }",
                vec![json!({"type": "string", "value": self.selector})],
                "none",
            )
            .await?;
        let mut out = Vec::new();
        for node in r["value"].as_array().into_iter().flatten() {
            out.push(self.page.element_handle_of(&context, node.clone()).await?);
        }
        Ok(out)
    }

    /// `locator.evaluateHandle(fn)`: `fn(element)` as a handle.
    pub async fn evaluate_handle(&self, function: &str) -> Result<JsHandle> {
        self.element_handle().await?.evaluate_handle(function).await
    }
}

impl JsHandle {
    /// This value as a BiDi argument.
    fn argument(&self) -> Value {
        if let Some(id) = self.remote.get("sharedId") {
            return json!({"sharedId": id});
        }
        if let Some(h) = self.remote.get("handle") {
            return json!({"handle": h});
        }
        self.remote.clone()
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
        let arg = serde_json::to_string(&arg)?;
        let r = self
            .page
            .call_function_raw(
                self.target.clone(),
                &call_source(function, true),
                vec![self.argument(), json!({"type": "string", "value": arg})],
                "none",
            )
            .await?;
        let v = match r["value"].as_str() {
            Some(text) => serde_json::from_str(text)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}")))?,
            None => Value::Null,
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
        let arg = serde_json::to_string(&arg)?;
        let remote = self
            .page
            .call_function_raw(
                self.target.clone(),
                &call_source(function, false),
                vec![self.argument(), json!({"type": "string", "value": arg})],
                "root",
            )
            .await?;
        Ok(JsHandle {
            page: self.page.clone(),
            target: self.target.clone(),
            remote,
        })
    }

    /// `jsHandle.jsonValue()`.
    pub async fn json_value(&self) -> Result<Value> {
        if self.remote.get("handle").is_none() && self.remote.get("sharedId").is_none() {
            return Ok(remote_value_to_json(&self.remote));
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
        if self.remote["type"] != "node" {
            return None;
        }
        let context = self.target["context"].as_str()?.to_owned();
        let page = self.page.clone();
        let remote = self.remote.clone();
        // Registered with the injected script on first use.
        Some(ElementHandle {
            handle: JsHandle {
                page,
                target: json!({"context": context}),
                remote,
            },
            context,
            id: String::new(),
        })
    }

    /// `jsHandle.dispose()`: releases the reference.
    pub async fn dispose(&self) -> Result<()> {
        if let Some(h) = self.remote.get("handle") {
            self.page
                .bidi_call(
                    "script.disown",
                    json!({"handles": [h], "target": self.target}),
                )
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
    /// A locator that matches exactly this element.
    async fn locator(&self) -> Result<Locator> {
        let id = if self.id.is_empty() {
            self.handle
                .page
                .element_handle_of(&self.context, self.handle.remote.clone())
                .await?
                .id
        } else {
            self.id.clone()
        };
        Ok(Locator {
            page: self.handle.page.clone(),
            root: Some(self.context.clone()),
            frames: Vec::new(),
            selector: format!("chromiumoxide-handle={id}"),
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
        let deadline = self.handle.page.now() + self.handle.page.timeout();
        loop {
            let v = locator
                .read_value(
                    "async (i, node, state) => { const s = await i.checkElementStates(node, [state], true); \
                     if (s === 'error:notconnected') throw i.createStacklessError('Element is not attached to the DOM'); \
                     return !s; }",
                    &[json!(state)],
                )
                .await?;
            if v == Value::Bool(true) {
                return Ok(());
            }
            if self.handle.page.now() >= deadline {
                return Err(Error::Timeout {
                    ms: self.handle.page.timeout().as_millis() as u64,
                    action: format!("waiting for element to be {state}"),
                });
            }
            self.handle.page.sleep(Duration::from_millis(16)).await;
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
        let r = self
            .handle
            .page
            .call_function_raw(
                json!({"context": self.context, "sandbox": UTILITY}),
                "(e) => e.contentWindow || null",
                vec![self.handle.argument()],
                "none",
            )
            .await?;
        let Some(context) = r.pointer("/value/context").and_then(Value::as_str) else {
            return Ok(None);
        };
        Ok(self
            .handle
            .page
            .frames()
            .await?
            .into_iter()
            .find(|f| f.context == context))
    }

    /// `elementHandle.ownerFrame()`: the frame the element is in.
    pub async fn owner_frame(&self) -> Result<Option<Frame>> {
        Ok(self
            .handle
            .page
            .frames()
            .await?
            .into_iter()
            .find(|f| f.context == self.context))
    }
}
