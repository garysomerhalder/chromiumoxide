//! Page actions: setContent, waitForFunction, addStyleTag, content, title.

use std::future::IntoFuture;
use std::path::PathBuf;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::{Value, json};

use crate::application::page::Page;
use crate::application::wait::wait_until;
use crate::domain::error::{Error, Result};
use crate::domain::frame::{LifecycleEvent, World};

/// Replaces the document and waits for it to load, then for its images and fonts.
const SET_CONTENT: &str =
    "function(html) { document.open(); document.write(html); document.close(); }";

const WAIT_FOR_ASSETS: &str = "async function() {
  await Promise.all([...document.images].map(img => img.complete ? null
    : new Promise(resolve => { img.addEventListener('load', resolve, { once: true }); img.addEventListener('error', resolve, { once: true }); })));
  await document.fonts.ready;
}";

const CONTENT: &str = "function() {
  let html = '';
  if (document.doctype) html = new XMLSerializer().serializeToString(document.doctype);
  if (document.documentElement) html += document.documentElement.outerHTML;
  return html;
}";

const ADD_STYLE: &str = "async function(content, url) {
  if (url) {
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = url;
    const loaded = new Promise((resolve, reject) => {
      link.onload = resolve;
      link.onerror = () => reject(new Error('Failed to load stylesheet ' + url));
    });
    document.head.appendChild(link);
    await loaded;
    return;
  }
  const style = document.createElement('style');
  style.type = 'text/css';
  style.appendChild(document.createTextNode(content));
  const loaded = new Promise((resolve, reject) => {
    style.onload = resolve;
    style.onerror = reject;
  });
  (document.head || document.documentElement).appendChild(style);
  await loaded;
}";

/// How `waitForFunction` re-checks its predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Polling {
    /// Every animation frame (Playwright's default).
    #[default]
    Raf,
    /// Every interval.
    Interval(Duration),
}

/// What `addStyleTag` adds (Playwright's `{ content | url | path }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StyleTag {
    Content(String),
    Url(String),
    Path(PathBuf),
}

impl StyleTag {
    pub fn content(css: &str) -> Self {
        Self::Content(css.to_owned())
    }
    pub fn url(url: &str) -> Self {
        Self::Url(url.to_owned())
    }
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self::Path(path.into())
    }
}

impl Page {
    /// Playwright's `page.setContent(html)`: replaces the document, waits for
    /// `load` (or the chosen state), then for images and fonts.
    pub fn set_content(&self, html: &str) -> SetContent<'_> {
        SetContent {
            page: self,
            html: html.to_owned(),
            wait_until: LifecycleEvent::Load,
            timeout: None,
        }
    }

    /// Playwright's `page.waitForFunction(fn | expression)`; resolves with the
    /// first truthy value.
    pub fn wait_for_function(&self, source: &str) -> WaitForFunction<'_> {
        WaitForFunction {
            page: self,
            source: source.to_owned(),
            arg: Value::Null,
            polling: Polling::Raf,
            timeout: None,
        }
    }

    /// Playwright's `page.addStyleTag(...)`; waits until the stylesheet loaded.
    pub async fn add_style_tag(&self, tag: StyleTag) -> Result<()> {
        let (content, url) = match tag {
            StyleTag::Content(c) => (Value::String(c), Value::Null),
            StyleTag::Url(u) => (Value::Null, Value::String(u)),
            StyleTag::Path(p) => {
                let bytes = self.inner.shared.services.fs.read(&p)?;
                let mut css = String::from_utf8_lossy(&bytes).into_owned();
                css.push_str(&format!(
                    "\n/*# sourceURL={}*/",
                    p.display().to_string().replace('\n', "")
                ));
                (Value::String(css), Value::Null)
            }
        };
        self.call_function(
            World::Main,
            ADD_STYLE,
            &[content, url],
            self.default_timeout(),
            "adding a style tag",
        )
        .await?;
        Ok(())
    }

    /// The full HTML of the document, doctype included.
    pub async fn content(&self) -> Result<String> {
        let v = self
            .call_function(
                World::Utility,
                CONTENT,
                &[],
                self.default_timeout(),
                "reading content",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    /// `document.title`.
    pub async fn title(&self) -> Result<String> {
        let v = self
            .call_function(
                World::Utility,
                "function() { return document.title }",
                &[],
                self.default_timeout(),
                "reading the title",
            )
            .await?;
        Ok(v.as_str().unwrap_or_default().to_owned())
    }

    async fn run_set_content(
        &self,
        html: &str,
        wait: LifecycleEvent,
        timeout: Duration,
    ) -> Result<()> {
        let clock = self.inner.shared.services.clock.clone();
        let start = clock.now_ms();
        let opened_before = self.inner.state.lock().unwrap().documents_opened;
        self.call_function(
            World::Utility,
            SET_CONTENT,
            &[Value::String(html.to_owned())],
            timeout,
            "setting content",
        )
        .await?;
        let remaining = |clock: &dyn crate::ports::Clock| {
            timeout.saturating_sub(Duration::from_millis(clock.now_ms() - start))
        };
        let inner = &self.inner;
        wait_until(
            &*clock,
            &inner.changed,
            remaining(&*clock),
            "setting content, waiting for load",
            || {
                if self.is_closed() {
                    return Some(Err(Error::TargetClosed("page has been closed".into())));
                }
                let st = inner.state.lock().unwrap();
                let main = st.tree.main_frame()?;
                (st.documents_opened > opened_before && main.has(wait)).then_some(Ok(()))
            },
        )
        .await?;
        self.call_function(
            World::Utility,
            WAIT_FOR_ASSETS,
            &[],
            remaining(&*clock),
            "waiting for images and fonts",
        )
        .await?;
        Ok(())
    }

    async fn run_wait_for_function(
        &self,
        source: &str,
        arg: &Value,
        polling: Polling,
        timeout: Duration,
    ) -> Result<Value> {
        let source = source.trim().trim_end_matches(';');
        let declaration = format!(
            "async function(__arg, __polling, __timeout) {{
  const __make = () => (
{source}
  );
  const __check = async () => {{ let v = __make(); if (typeof v === 'function') v = v(__arg); return await v; }};
  return await new Promise((resolve, reject) => {{
    let done = false;
    const timer = setTimeout(() => {{ done = true; resolve({{ timedOut: true }}); }}, __timeout);
    const next = async () => {{
      if (done) return;
      let v;
      try {{ v = await __check(); }} catch (e) {{ done = true; clearTimeout(timer); reject(e); return; }}
      if (v) {{ done = true; clearTimeout(timer); resolve({{ value: v }}); return; }}
      if (__polling === 'raf') requestAnimationFrame(next); else setTimeout(next, __polling);
    }};
    next();
  }});
}}"
        );
        let polling = match polling {
            Polling::Raf => json!("raf"),
            Polling::Interval(d) => json!(d.as_millis() as u64),
        };
        let timeout_err = || Error::Timeout {
            ms: timeout.as_millis() as u64,
            action: "waiting for function".into(),
        };
        let r = self
            .call_function(
                World::Main,
                &declaration,
                &[arg.clone(), polling, json!(timeout.as_millis() as u64)],
                timeout,
                "waiting for function",
            )
            .await
            .map_err(|e| {
                if matches!(e, Error::Timeout { .. }) {
                    timeout_err()
                } else {
                    e
                }
            })?;
        if r.get("timedOut").and_then(Value::as_bool) == Some(true) {
            return Err(timeout_err());
        }
        Ok(r.get("value").cloned().unwrap_or(Value::Null))
    }
}

/// A pending `page.setContent()`.
#[must_use = "does nothing until awaited"]
pub struct SetContent<'a> {
    page: &'a Page,
    html: String,
    wait_until: LifecycleEvent,
    timeout: Option<Duration>,
}

impl<'a> SetContent<'a> {
    pub fn wait_until(mut self, event: LifecycleEvent) -> Self {
        self.wait_until = event;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for SetContent<'a> {
    type Output = Result<()>;
    type IntoFuture = BoxFuture<'a, Result<()>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
            self.page
                .run_set_content(&self.html, self.wait_until, timeout)
                .await
        }
        .boxed()
    }
}

/// A pending `page.waitForFunction()`.
#[must_use = "does nothing until awaited"]
pub struct WaitForFunction<'a> {
    page: &'a Page,
    source: String,
    arg: Value,
    polling: Polling,
    timeout: Option<Duration>,
}

impl<'a> WaitForFunction<'a> {
    /// Argument passed to the predicate function.
    pub fn arg<A: Serialize>(mut self, arg: A) -> Self {
        self.arg = serde_json::to_value(arg).unwrap_or(Value::Null);
        self
    }
    pub fn polling(mut self, polling: Polling) -> Self {
        self.polling = polling;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl<'a> IntoFuture for WaitForFunction<'a> {
    type Output = Result<Value>;
    type IntoFuture = BoxFuture<'a, Result<Value>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
            self.page
                .run_wait_for_function(&self.source, &self.arg, self.polling, timeout)
                .await
        }
        .boxed()
    }
}
