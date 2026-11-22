//! JS handles: references to in-page objects (Playwright's `JSHandle`).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::js;
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;

/// A reference to a JavaScript object in the page. Cheap to clone.
#[derive(Clone)]
pub struct JsHandle {
    page: Page,
    object_id: Option<String>,
    /// Primitive values (no object id) are kept by value.
    value: Value,
    subtype: Option<String>,
    disposed: Arc<AtomicBool>,
    /// A worker's session, when the object lives in a worker.
    session: Option<String>,
}

impl std::fmt::Debug for JsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JsHandle")
            .field("object_id", &self.object_id)
            .field("subtype", &self.subtype)
            .finish()
    }
}

impl JsHandle {
    /// A handle to an object in a worker reached through `session`.
    pub(crate) fn from_worker(page: &Page, session: &str, remote: &Value) -> JsHandle {
        let mut h = JsHandle::from_remote(page, remote);
        h.session = Some(session.to_owned());
        h
    }

    fn child(&self, remote: &Value) -> JsHandle {
        let mut h = JsHandle::from_remote(&self.page, remote);
        h.session = self.session.clone();
        h
    }

    async fn send(&self, method: &str, params: Value) -> Result<Value> {
        match &self.session {
            Some(sid) => {
                self.page
                    .inner
                    .shared
                    .cdp
                    .call(Some(sid), method, params)
                    .await
            }
            None => self.page.send(method, params).await,
        }
    }

    pub(crate) fn from_remote(page: &Page, remote: &Value) -> JsHandle {
        JsHandle {
            page: page.clone(),
            object_id: remote
                .get("objectId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            value: js::result_value(&json!({"result": remote})).unwrap_or(Value::Null),
            subtype: remote
                .get("subtype")
                .and_then(Value::as_str)
                .map(str::to_owned),
            disposed: Arc::new(AtomicBool::new(false)),
            session: None,
        }
    }

    fn live(&self) -> Result<()> {
        if self.disposed.load(Ordering::SeqCst) {
            return Err(Error::Evaluation("JSHandle is disposed".into()));
        }
        Ok(())
    }

    async fn call(&self, function: &str, arg: Option<Value>, by_value: bool) -> Result<Value> {
        self.live()?;
        let source = function.trim().trim_end_matches(';');
        let declaration = format!(
            "async function(arg) {{ const __v = (\n{source}\n); return typeof __v === 'function' ? await __v(this, arg) : __v; }}"
        );
        let mut params = json!({
            "functionDeclaration": declaration,
            "arguments": [{"value": arg.unwrap_or(Value::Null)}],
            "returnByValue": by_value,
            "awaitPromise": true,
            "userGesture": true,
        });
        match &self.object_id {
            Some(id) => params["objectId"] = json!(id),
            None => {
                // Primitive: evaluate against a fresh copy in the main world.
                let context = self
                    .page
                    .main_context(World::Main, self.page.default_timeout())
                    .await?;
                params["executionContextId"] = json!(context);
                params["functionDeclaration"] = json!(format!(
                    "async function(arg) {{ const __this = {}; const __v = (\n{source}\n); return typeof __v === 'function' ? await __v(__this, arg) : __v; }}",
                    self.value
                ));
            }
        }
        let r = self.send("Runtime.callFunctionOn", params).await?;
        if by_value {
            js::result_value(&r)
        } else {
            js::result_value(&r)?;
            Ok(r["result"].clone())
        }
    }

    /// `handle.evaluate(fn)`: `fn(object)`.
    pub async fn evaluate<T: DeserializeOwned>(&self, function: &str) -> Result<T> {
        let v = self.call(function, None, true).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `handle.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        function: &str,
        arg: A,
    ) -> Result<T> {
        let v = self
            .call(function, Some(serde_json::to_value(arg)?), true)
            .await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `handle.evaluateHandle(fn)`.
    pub async fn evaluate_handle(&self, function: &str) -> Result<JsHandle> {
        let remote = self.call(function, None, false).await?;
        Ok(self.child(&remote))
    }

    /// `handle.jsonValue()`.
    pub async fn json_value<T: DeserializeOwned>(&self) -> Result<T> {
        self.live()?;
        let v = match &self.object_id {
            Some(_) => self.call("o => o", None, true).await?,
            None => self.value.clone(),
        };
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize value: {e}")))
    }

    /// `handle.getProperty(name)`.
    pub async fn get_property(&self, name: &str) -> Result<JsHandle> {
        let remote = self
            .call("(o, name) => o[name]", Some(json!(name)), false)
            .await?;
        Ok(self.child(&remote))
    }

    /// `handle.getProperties()`: own enumerable properties.
    pub async fn get_properties(&self) -> Result<BTreeMap<String, JsHandle>> {
        self.live()?;
        let Some(id) = &self.object_id else {
            return Ok(BTreeMap::new());
        };
        let r = self
            .send(
                "Runtime.getProperties",
                json!({"objectId": id, "ownProperties": true}),
            )
            .await?;
        let mut out = BTreeMap::new();
        for p in r
            .get("result")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if p.get("enumerable").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            if let (Some(name), Some(value)) =
                (p.get("name").and_then(Value::as_str), p.get("value"))
            {
                out.insert(name.to_owned(), self.child(value));
            }
        }
        Ok(out)
    }

    /// `handle.asElement()`: `Some` for DOM elements.
    pub fn as_element(&self) -> Option<JsHandle> {
        // Like Playwright, any DOM node (subtype "node") yields an element handle.
        (self.subtype.as_deref() == Some("node") && self.object_id.is_some()).then(|| self.clone())
    }

    /// `handle.dispose()`.
    pub async fn dispose(&self) -> Result<()> {
        if self.disposed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if let Some(id) = &self.object_id {
            let _ = self
                .send("Runtime.releaseObject", json!({"objectId": id}))
                .await;
        }
        Ok(())
    }
}

impl Page {
    /// `page.evaluateHandle(expression | function)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        let context = self
            .main_context(World::Main, self.default_timeout())
            .await?;
        let r = self
            .send(
                "Runtime.callFunctionOn",
                json!({"functionDeclaration": js::function_declaration(source), "executionContextId": context,
                       "arguments": [], "returnByValue": false, "awaitPromise": true, "userGesture": true}),
            )
            .await?;
        js::result_value(&r)?;
        Ok(JsHandle::from_remote(self, &r["result"]))
    }
}
