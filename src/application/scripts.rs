//! Use cases: init scripts, script tags and exposed functions (bindings).

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::page::Page;
use crate::domain::error::{Error, Result};
use crate::domain::frame::World;

/// A Rust function callable from the page.
pub type BindingFn = Arc<
    dyn Fn(BindingSource, Vec<Value>) -> BoxFuture<'static, std::result::Result<Value, String>>
        + Send
        + Sync,
>;

/// Where a binding call came from (Playwright's `source` argument).
#[derive(Debug, Clone)]
pub struct BindingSource {
    pub page: Page,
    pub frame: crate::application::frame::Frame,
}

const BINDING_PREFIX: &str = "__chromiumoxide_binding_";

fn binding_name(name: &str) -> String {
    format!("{BINDING_PREFIX}{name}")
}

fn deliver_name(name: &str) -> String {
    format!("__chromiumoxide_deliver_{name}")
}

/// Installs `window[name]` that forwards calls to the CDP binding.
fn binding_wrapper(name: &str) -> String {
    let name_js = serde_json::to_string(name).expect("string");
    let binding_js = serde_json::to_string(&binding_name(name)).expect("string");
    let deliver_js = serde_json::to_string(&deliver_name(name)).expect("string");
    format!(
        "(() => {{
  const name = {name_js};
  const binding = globalThis[{binding_js}];
  if (!binding || globalThis[{deliver_js}]) return;
  const callbacks = new Map();
  let lastId = 0;
  globalThis[{deliver_js}] = (id, ok, value) => {{
    const cb = callbacks.get(id);
    callbacks.delete(id);
    if (!cb) return;
    if (ok) cb.resolve(value); else cb.reject(new Error(value));
  }};
  globalThis[name] = (...args) => new Promise((resolve, reject) => {{
    const id = ++lastId;
    callbacks.set(id, {{ resolve, reject }});
    binding(JSON.stringify({{ id, args }}));
  }});
}})();"
    )
}

/// Commands that apply a context's init scripts and bindings to a new page.
pub(crate) fn context_script_commands(context: &ContextInner) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for name in context.bindings.lock().unwrap().keys() {
        out.extend(binding_commands(name));
    }
    for source in context.init_scripts.lock().unwrap().iter() {
        out.push((
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": source, "runImmediately": true}),
        ));
    }
    out
}

fn binding_commands(name: &str) -> Vec<(&'static str, Value)> {
    vec![
        ("Runtime.addBinding", json!({"name": binding_name(name)})),
        (
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": binding_wrapper(name), "runImmediately": true}),
        ),
    ]
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

fn init_source(source: &str, arg: Option<Value>) -> String {
    match arg {
        Some(arg) => format!("({})({});", source.trim().trim_end_matches(';'), arg),
        None => source.to_owned(),
    }
}

/// What `addScriptTag` adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptTag {
    source: ScriptSource,
    module: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScriptSource {
    Content(String),
    Url(String),
    Path(PathBuf),
}

impl ScriptTag {
    pub fn content(js: &str) -> Self {
        Self {
            source: ScriptSource::Content(js.to_owned()),
            module: false,
        }
    }
    pub fn url(url: &str) -> Self {
        Self {
            source: ScriptSource::Url(url.to_owned()),
            module: false,
        }
    }
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self {
            source: ScriptSource::Path(path.into()),
            module: false,
        }
    }
    /// `type="module"`.
    pub fn module(mut self) -> Self {
        self.module = true;
        self
    }
}

const ADD_SCRIPT: &str = "async function(content, url, type) {
  const script = document.createElement('script');
  if (type) script.type = type;
  if (url) {
    script.src = url;
    const loaded = new Promise((resolve, reject) => {
      script.onload = resolve;
      script.onerror = () => reject(new Error('Failed to load script at ' + url));
    });
    document.head.appendChild(script);
    await loaded;
    return;
  }
  const key = '__chromiumoxide_script_' + Math.random().toString(36).slice(2);
  const ran = new Promise(resolve => { globalThis[key] = resolve; });
  script.text = content + '\\n;globalThis[' + JSON.stringify(key) + ']();';
  let error = null;
  script.onerror = e => { error = e; };
  (document.head || document.documentElement).appendChild(script);
  if (error) throw error;
  await ran;
  delete globalThis[key];
}";

impl Page {
    /// Playwright's `page.addInitScript(script)`: runs before page scripts in
    /// every document this page loads from now on.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": init_source(source, None)}),
        )
        .await?;
        Ok(())
    }

    /// `page.addInitScript(fn, arg)`.
    pub async fn add_init_script_with<A: Serialize>(&self, function: &str, arg: A) -> Result<()> {
        let source = init_source(function, Some(serde_json::to_value(arg)?));
        self.send(
            "Page.addScriptToEvaluateOnNewDocument",
            json!({"source": source}),
        )
        .await?;
        Ok(())
    }

    /// Playwright's `page.addScriptTag(...)`; waits until the script ran.
    pub async fn add_script_tag(&self, tag: ScriptTag) -> Result<()> {
        let (content, url) = match tag.source {
            ScriptSource::Content(c) => (json!(c), Value::Null),
            ScriptSource::Url(u) => (Value::Null, json!(u)),
            ScriptSource::Path(p) => {
                let bytes = self.inner.shared.services.fs.read(&p)?;
                let mut js = String::from_utf8_lossy(&bytes).into_owned();
                js.push_str(&format!(
                    "\n//# sourceURL={}",
                    p.display().to_string().replace('\n', "")
                ));
                (json!(js), Value::Null)
            }
        };
        let kind = if tag.module {
            json!("module")
        } else {
            Value::Null
        };
        self.call_function(
            World::Main,
            ADD_SCRIPT,
            &[content, url, kind],
            self.default_timeout(),
            "adding a script tag",
        )
        .await?;
        Ok(())
    }

    /// Playwright's `page.exposeFunction(name, fn)`: `window[name](...args)`
    /// returns a promise of `fn(args)`; an `Err(message)` rejects it.
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
            let mut bindings = self.inner.bindings.lock().unwrap();
            if taken_by_context || bindings.contains_key(name) {
                return Err(Error::InvalidArgument(format!(
                    "Function \"{name}\" has been already registered"
                )));
            }
            bindings.insert(name.to_owned(), callback);
        }
        for (method, params) in binding_commands(name) {
            self.send(method, params).await?;
        }
        Ok(())
    }

    pub(crate) fn on_binding_called(&self, p: &Value) {
        let Some(full) = p.get("name").and_then(Value::as_str) else {
            return;
        };
        let Some(name) = full.strip_prefix(BINDING_PREFIX) else {
            return;
        };
        let callback = self
            .inner
            .bindings
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .or_else(|| {
                self.inner
                    .context
                    .upgrade()
                    .and_then(|c| c.bindings.lock().unwrap().get(name).cloned())
            });
        let Some(callback) = callback else { return };
        let context_id = p.get("executionContextId").cloned().unwrap_or(Value::Null);
        let payload: Value = p
            .get("payload")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        let id = payload.get("id").cloned().unwrap_or(Value::Null);
        let args = payload
            .get("args")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let page = self.clone();
        let frame_id = context_id
            .as_i64()
            .and_then(|id| {
                let st = self.inner.state.lock().unwrap();
                st.tree.frame_for_context(id).map(|f| f.id.clone())
            })
            .unwrap_or_else(|| self.main_frame().id().to_owned());
        let source = BindingSource {
            page: self.clone(),
            frame: self.frame_by_id(&frame_id),
        };
        let deliver = serde_json::to_string(&deliver_name(name)).expect("string");
        tokio::spawn(async move {
            let (ok, value) = match callback(source, args).await {
                Ok(v) => (true, v),
                Err(e) => (false, Value::String(e)),
            };
            let expression = format!("globalThis[{deliver}]({id}, {ok}, {value})");
            let _ = page
                .send(
                    "Runtime.evaluate",
                    json!({"expression": expression, "contextId": context_id}),
                )
                .await;
        });
    }
}

impl BrowserContext {
    /// Playwright's `context.addInitScript(script)`, for every page of the context.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.inner
            .init_scripts
            .lock()
            .unwrap()
            .push(source.to_owned());
        for page in self.pages() {
            page.send(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": source}),
            )
            .await?;
        }
        Ok(())
    }

    /// Playwright's `context.exposeFunction(name, fn)`, for every page of the context.
    pub async fn expose_function<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_context_binding(name, boxed(f)).await
    }

    /// Playwright's `context.exposeBinding(name, fn)`: like `expose_function`,
    /// with the calling page and frame.
    pub async fn expose_binding<F, Fut>(&self, name: &str, f: F) -> Result<()>
    where
        F: Fn(BindingSource, Vec<Value>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = std::result::Result<Value, String>> + Send + 'static,
    {
        self.register_context_binding(name, boxed_binding(f)).await
    }

    async fn register_context_binding(&self, name: &str, callback: BindingFn) -> Result<()> {
        {
            let mut bindings = self.inner.bindings.lock().unwrap();
            let on_page = self
                .pages()
                .iter()
                .any(|p| p.inner.bindings.lock().unwrap().contains_key(name));
            if on_page || bindings.contains_key(name) {
                return Err(Error::InvalidArgument(format!(
                    "Function \"{name}\" has been already registered"
                )));
            }
            bindings.insert(name.to_owned(), callback);
        }
        for page in self.pages() {
            for (method, params) in binding_commands(name) {
                page.send(method, params).await?;
            }
        }
        Ok(())
    }
}
