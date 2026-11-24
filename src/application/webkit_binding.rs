//! `exposeFunction` / `exposeBinding` on WebKit, as Playwright's `WKPage`:
//! one `Runtime.addBinding` per page target; an init script (and, for open
//! documents, the same script run once per frame) defines `window[name]`,
//! which calls the binding with `{ name, id, args }` as JSON. WebKit
//! reports the call as `Runtime.bindingCalled`; the result goes back to
//! the calling execution context.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::webkit::{BrowserContext, Page};
use crate::application::webkit_frame::Frame;
use crate::domain::error::{Error, Result};

/// The WebKit binding every exposed function calls.
pub(crate) const BINDING: &str = "__chromiumoxide_binding";

/// Where a binding call came from (Playwright's `source` argument).
#[derive(Debug, Clone)]
pub struct BindingSource {
    pub page: Page,
    pub frame: Frame,
}

/// An exposed binding, boxed.
pub(crate) type BindingFn = Arc<
    dyn Fn(BindingSource, Vec<Value>) -> BoxFuture<'static, std::result::Result<Value, String>>
        + Send
        + Sync,
>;

/// The bindings of a page or context, by name.
pub(crate) type Bindings = std::sync::Mutex<HashMap<String, BindingFn>>;

/// Defines `window[name]` (calls the WebKit binding with JSON).
fn install_script(name: &str) -> String {
    format!(
        "(() => {{ const name = {name}; \
         const pending = globalThis.__chromiumoxide_binding_pending || \
           (globalThis.__chromiumoxide_binding_pending = new Map()); \
         if (!globalThis.__chromiumoxide_binding_done) globalThis.__chromiumoxide_binding_done = (id, ok, value) => {{ \
           const p = pending.get(id); if (!p) return; pending.delete(id); \
           if (ok) p.resolve(value); else p.reject(new Error(value)); }}; \
         const binding = globalThis.{BINDING}; \
         globalThis[name] = (...args) => new Promise((resolve, reject) => {{ \
           const id = Math.random().toString(36).slice(2) + '-' + Date.now(); \
           pending.set(id, {{ resolve, reject }}); \
           binding(JSON.stringify({{ name, id, args }})); }}); }})()",
        name = Value::String(name.to_owned())
    )
}

fn boxed<F, Fut>(f: F) -> BindingFn
where
    F: Fn(Vec<Value>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
{
    Arc::new(move |_source, args| f(args).boxed())
}

fn boxed_binding<F, Fut>(f: F) -> BindingFn
where
    F: Fn(BindingSource, Vec<Value>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
{
    Arc::new(move |source, args| f(source, args).boxed())
}

fn already(name: &str) -> Error {
    Error::InvalidArgument(format!("Function \"{name}\" has been already registered"))
}

impl Page {
    /// Playwright's `page.exposeFunction(name, fn)`: `window[name](...)`
    /// in every frame of this page calls `fn` and resolves with its result.
    pub async fn expose_function<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_binding(name, boxed(f)).await
    }

    /// Playwright's `page.exposeBinding(name, fn)`: like `expose_function`,
    /// and `fn` also receives the calling page and frame.
    pub async fn expose_binding<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(BindingSource, Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_binding(name, boxed_binding(f)).await
    }

    async fn register_binding(&self, name: &str, callback: BindingFn) -> Result<()> {
        let taken_by_context = self
            .inner
            .context
            .upgrade()
            .is_some_and(|c| c.bindings.lock().unwrap().contains_key(name));
        {
            let mut own = self.inner.bindings.lock().unwrap();
            if taken_by_context || own.contains_key(name) {
                return Err(already(name));
            }
            own.insert(name.to_owned(), callback);
        }
        self.add_init_script(&install_script(name)).await?;
        self.install_in_frames(name).await;
        Ok(())
    }

    /// Defines the binding in the documents open now.
    pub(crate) async fn install_in_frames(&self, name: &str) {
        for frame in self.frames().await.unwrap_or_default() {
            let _ = self
                .evaluate_value_in(&install_script(name), None, frame.scope())
                .await;
        }
    }

    /// `Runtime.bindingCalled`: runs the binding and answers the calling
    /// execution context.
    pub(crate) fn on_binding_called(&self, p: &Value) {
        if p["name"] != BINDING {
            return;
        }
        let Some(data) = p["argument"]
            .as_str()
            .and_then(|d| serde_json::from_str::<Value>(d).ok())
        else {
            return;
        };
        let name = data["name"].as_str().unwrap_or_default().to_owned();
        let callback = self
            .inner
            .bindings
            .lock()
            .unwrap()
            .get(&name)
            .cloned()
            .or_else(|| {
                self.inner
                    .context
                    .upgrade()
                    .and_then(|c| c.bindings.lock().unwrap().get(&name).cloned())
            });
        let Some(callback) = callback else {
            return;
        };
        let id = data["id"].as_str().unwrap_or_default().to_owned();
        let args = data["args"].as_array().cloned().unwrap_or_default();
        let context = p["contextId"].as_i64().unwrap_or_default();
        let page = self.clone();
        tokio::spawn(async move {
            // The calling frame: the one whose main world is `context`.
            let frame_id = page
                .inner
                .frame_worlds
                .lock()
                .unwrap()
                .iter()
                .find(|(_, w)| w.0 == Some(context))
                .map(|(f, _)| f.clone());
            let frames = page.frames().await.unwrap_or_default();
            let frame = frame_id
                .and_then(|id| frames.iter().find(|f| f.id == id).cloned())
                .unwrap_or_else(|| page.main_frame());
            let result = callback(
                BindingSource {
                    page: page.clone(),
                    frame,
                },
                args,
            )
            .await;
            let (ok, value) = match result {
                Ok(v) => (true, v),
                Err(e) => (false, Value::String(e)),
            };
            let expression = format!(
                "globalThis.__chromiumoxide_binding_done({}, {ok}, {})",
                Value::String(id),
                value
            );
            let _ = page
                .target_call(
                    "Runtime.evaluate",
                    json!({"expression": expression, "contextId": context}),
                )
                .await;
        });
    }
}

impl BrowserContext {
    /// Playwright's `context.exposeFunction(name, fn)`: in every page of
    /// the context.
    pub async fn expose_function<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_context_binding(name, boxed(f)).await
    }

    /// Playwright's `context.exposeBinding(name, fn)`: like
    /// `expose_function`, with the calling page and frame.
    pub async fn expose_binding<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(BindingSource, Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_context_binding(name, boxed_binding(f)).await
    }

    async fn register_context_binding(&self, name: &str, callback: BindingFn) -> Result<()> {
        {
            let pages_have = self
                .live_pages()
                .iter()
                .any(|p| p.inner.bindings.lock().unwrap().contains_key(name));
            let mut own = self.inner.bindings.lock().unwrap();
            if pages_have || own.contains_key(name) {
                return Err(already(name));
            }
            own.insert(name.to_owned(), callback);
        }
        self.add_init_script(&install_script(name)).await?;
        for page in self.live_pages() {
            page.install_in_frames(name).await;
        }
        Ok(())
    }
}
