//! Dedicated web workers of a page (Playwright's `Worker`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::events::{ConsoleMessage, EventWaiter, PageEvent};
use crate::application::js;
use crate::application::page::Page;
use crate::domain::console::{console_text, location};
use crate::domain::error::{Error, Result};
use crate::ports::RawEvent;

/// A web worker. Cheap to clone.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<WorkerInner>,
}

struct WorkerInner {
    page: Page,
    session_id: String,
    url: String,
    /// The worker's execution context, once created.
    context: watch::Sender<Option<i64>>,
    closed: AtomicBool,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("url", &self.inner.url)
            .finish()
    }
}

impl PartialEq for Worker {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Worker {
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Waits until the worker is terminated.
    pub async fn wait_for_close(&self) {
        let mut rx = self.inner.context.subscribe();
        while !self.is_closed() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// `worker.evaluate(pageFunction)`: a function or an expression.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.call(source, None).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `worker.evaluate(pageFunction, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self.call(source, Some(arg)).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    async fn call(&self, source: &str, arg: Option<Value>) -> Result<Value> {
        let page = &self.inner.page;
        let rx = self.inner.context.subscribe();
        let context_id = {
            let mut rx = rx;
            let timeout = page.default_timeout();
            let wait = async {
                loop {
                    if self.is_closed() {
                        return Err(Error::TargetClosed("worker has been closed".into()));
                    }
                    if let Some(id) = *rx.borrow() {
                        return Ok(id);
                    }
                    if rx.changed().await.is_err() {
                        return Err(Error::TargetClosed("worker has been closed".into()));
                    }
                }
            };
            crate::application::wait::with_timeout(
                &*page.inner.shared.services.clock,
                timeout,
                "waiting for the worker",
                wait,
            )
            .await?
        };
        let args: Vec<Value> = arg.into_iter().collect();
        let params = js::call_params(&js::function_declaration(source), context_id, &args);
        let r = page
            .inner
            .shared
            .cdp
            .call(
                Some(&self.inner.session_id),
                "Runtime.callFunctionOn",
                params,
            )
            .await?;
        js::result_value(&r)
    }

    fn mark_closed(&self) {
        if !self.inner.closed.swap(true, Ordering::SeqCst) {
            self.inner.context.send_modify(|_| {});
        }
    }
}

impl Page {
    /// A worker target attached under this page.
    pub(crate) fn attach_worker(&self, session_id: String, url: String) {
        let (context, _) = watch::channel(None);
        let worker = Worker {
            inner: Arc::new(WorkerInner {
                page: self.clone(),
                session_id: session_id.clone(),
                url,
                context,
                closed: AtomicBool::new(false),
            }),
        };
        self.inner.workers.lock().unwrap().push(worker.clone());
        let cdp = self.inner.shared.cdp.clone();
        let mut events = cdp.subscribe();
        let w = worker.clone();
        tokio::spawn(async move {
            while let Some(e) = events.next().await {
                if e.session_id.as_deref() != Some(w.inner.session_id.as_str()) {
                    continue;
                }
                w.handle_event(&e);
                if w.is_closed() {
                    break;
                }
            }
        });
        let sid = Some(session_id.as_str());
        let enable = cdp.call(sid, "Runtime.enable", json!({}));
        let run = cdp.call(sid, "Runtime.runIfWaitingForDebugger", json!({}));
        tokio::spawn(async move {
            let _ = futures::join!(enable, run);
        });
        self.emit(PageEvent::Worker(worker));
    }

    /// `Target.detachedFromTarget` for one of this page's workers.
    pub(crate) fn detach_worker(&self, session_id: &str) {
        let gone: Vec<Worker> = {
            let mut workers = self.inner.workers.lock().unwrap();
            let (gone, keep) = workers
                .drain(..)
                .partition(|w| w.inner.session_id == session_id);
            *workers = keep;
            gone
        };
        for w in gone {
            w.mark_closed();
        }
    }

    pub(crate) fn close_workers(&self) {
        let all: Vec<Worker> = self.inner.workers.lock().unwrap().drain(..).collect();
        for w in all {
            w.mark_closed();
        }
    }

    /// `page.workers()`.
    pub fn workers(&self) -> Vec<Worker> {
        self.inner.workers.lock().unwrap().clone()
    }

    /// `page.waitForEvent('worker')`.
    pub fn wait_for_worker(&self) -> EventWaiter<'_, Worker> {
        self.wait_for_event(|e| match e {
            PageEvent::Worker(w) => Some(w.clone()),
            _ => None,
        })
    }
}

impl Worker {
    fn handle_event(&self, e: &RawEvent) {
        let p = &e.params;
        match e.method.as_str() {
            "Runtime.executionContextCreated" => {
                if let Some(id) = p.pointer("/context/id").and_then(Value::as_i64) {
                    self.inner.context.send_replace(Some(id));
                }
            }
            "Runtime.consoleAPICalled" => {
                let args: Vec<Value> = p
                    .get("args")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let message = ConsoleMessage::from_worker(
                    p.get("type").and_then(Value::as_str).unwrap_or("log"),
                    console_text(&args),
                    location(p.get("stackTrace")),
                );
                self.inner.page.emit(PageEvent::Console(message));
            }
            _ => {}
        }
    }
}
