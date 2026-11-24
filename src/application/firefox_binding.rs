//! `exposeFunction` / `exposeBinding` on Firefox, over WebDriver BiDi
//! channels: a preload script (and, for open documents, the same function
//! called once per frame) defines `window[name]`, which sends the call
//! through a `script.message` channel; the result goes back to the calling
//! realm with `script.callFunction`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::application::firefox::{BrowserContext, Frame, Page};
use crate::domain::error::{Error, Result};

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

/// Defines `window[name]`; `channel` sends `{ name, id, args }` as JSON.
fn install_source(name: &str) -> String {
    format!(
        "(channel) => {{ const name = {name}; \
         const pending = globalThis.__chromiumoxide_binding_pending || \
           (globalThis.__chromiumoxide_binding_pending = new Map()); \
         if (!globalThis.__chromiumoxide_binding_done) globalThis.__chromiumoxide_binding_done = (id, ok, value) => {{ \
           const p = pending.get(id); if (!p) return; pending.delete(id); \
           if (ok) p.resolve(value); else p.reject(new Error(value)); }}; \
         globalThis[name] = (...args) => new Promise((resolve, reject) => {{ \
           const id = Math.random().toString(36).slice(2) + '-' + Date.now(); \
           pending.set(id, {{ resolve, reject }}); \
           channel(JSON.stringify({{ name, id, args }})); }}); }}",
        name = Value::String(name.to_owned())
    )
}

/// The channel of a page's bindings, and of a context's.
pub(crate) fn page_channel(context: &str) -> String {
    format!("chromiumoxide-binding:page:{context}")
}
pub(crate) fn context_channel(user_context: &str) -> String {
    format!("chromiumoxide-binding:context:{user_context}")
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
            .context()
            .is_some_and(|c| c.inner.bindings.lock().unwrap().contains_key(name));
        {
            let mut own = self.inner.bindings.lock().unwrap();
            if taken_by_context || own.contains_key(name) {
                return Err(already(name));
            }
            own.insert(name.to_owned(), callback);
        }
        let channel = page_channel(&self.main_context());
        self.bidi_call(
            "script.addPreloadScript",
            json!({"functionDeclaration": install_source(name),
                   "arguments": [{"type": "channel", "value": {"channel": channel}}],
                   "contexts": [self.main_context()]}),
        )
        .await?;
        self.install_in_frames(name, &channel).await;
        Ok(())
    }

    /// Defines the binding in the documents open now.
    pub(crate) async fn install_in_frames(&self, name: &str, channel: &str) {
        for frame in self.frames().await.unwrap_or_default() {
            let _ = self
                .call_function_raw(
                    json!({"context": frame.context}),
                    &install_source(name),
                    vec![json!({"type": "channel", "value": {"channel": channel}})],
                    "none",
                )
                .await;
        }
    }

    /// A `script.message` on a binding channel: runs the binding and
    /// answers the calling realm.
    pub(crate) fn binding_message(&self, p: &Value) {
        let channel = p["channel"].as_str().unwrap_or_default();
        let ctx = self.inner.owner.upgrade();
        let is_page = channel == page_channel(&self.main_context());
        let is_context = ctx
            .as_ref()
            .is_some_and(|c| channel == context_channel(&c.user_context));
        if !is_page && !is_context {
            return;
        }
        let Some(data) = p
            .pointer("/data/value")
            .and_then(Value::as_str)
            .and_then(|d| serde_json::from_str::<Value>(d).ok())
        else {
            return;
        };
        let id = data["id"].as_str().unwrap_or_default().to_owned();
        let name = data["name"].as_str().unwrap_or_default().to_owned();
        if is_context {
            // Every page of the context sees the message; one answers.
            let Some(ctx) = &ctx else {
                return;
            };
            let mut seen = ctx.binding_calls.lock().unwrap();
            if !seen.insert(id.clone()) {
                return;
            }
        }
        let callback = if is_page {
            self.inner.bindings.lock().unwrap().get(&name).cloned()
        } else {
            ctx.as_ref()
                .and_then(|c| c.bindings.lock().unwrap().get(&name).cloned())
        };
        let Some(callback) = callback else {
            return;
        };
        let realm = p
            .pointer("/source/realm")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let source_context = p
            .pointer("/source/context")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let args = data["args"].as_array().cloned().unwrap_or_default();
        let page = self.clone();
        tokio::spawn(async move {
            // The calling page: the one whose frames include the caller.
            let mut caller = None;
            let pages = match page.context() {
                Some(c) => c.pages(),
                None => vec![page.clone()],
            };
            for p in pages {
                if let Some(f) = p
                    .frames()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .find(|f| f.context == source_context)
                {
                    caller = Some((p, f));
                    break;
                }
            }
            let Some((caller_page, frame)) = caller else {
                return;
            };
            let result = callback(
                BindingSource {
                    page: caller_page.clone(),
                    frame,
                },
                args,
            )
            .await;
            let (ok, value) = match result {
                Ok(v) => (true, v),
                Err(e) => (false, Value::String(e)),
            };
            let _ = caller_page
                .call_function_raw(
                    json!({"realm": realm}),
                    "(id, ok, value) => globalThis.__chromiumoxide_binding_done(id, ok, JSON.parse(value))",
                    vec![
                        json!({"type": "string", "value": id}),
                        json!({"type": "boolean", "value": ok}),
                        json!({"type": "string", "value": value.to_string()}),
                    ],
                    "none",
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
        let channel = context_channel(&self.inner.user_context);
        self.inner
            .bidi
            .call(
                "script.addPreloadScript",
                json!({"functionDeclaration": install_source(name),
                       "arguments": [{"type": "channel", "value": {"channel": channel}}],
                       "userContexts": [self.inner.user_context]}),
            )
            .await?;
        for page in self.live_pages() {
            page.install_in_frames(name, &channel).await;
        }
        Ok(())
    }
}
