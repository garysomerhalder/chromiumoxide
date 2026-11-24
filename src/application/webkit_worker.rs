//! Dedicated workers on the WebKit engine (`page.workers()`), as
//! Playwright's `WKWorkers`: the page target reports each worker with
//! `Worker.workerCreated`; protocol messages to the worker travel in
//! `Worker.sendMessageToWorker` and come back in
//! `Worker.dispatchMessageFromWorker`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::channel::oneshot;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::webkit::{Page, PageInner};
use crate::application::webkit_handle::{JsHandle, Via};
use crate::domain::error::{Error, Result};

/// How long a worker may take to answer a command.
const ANSWER: Duration = Duration::from_secs(30);

/// A dedicated worker of a WebKit page. Cheap to clone.
#[derive(Clone)]
pub struct Worker {
    inner: Arc<WorkerInner>,
}

/// `worker.on(...)` events.
#[derive(Clone, Debug)]
pub enum WorkerEvent {
    /// The worker was terminated (or its page went away).
    Close,
}

struct WorkerInner {
    page: Weak<PageInner>,
    /// The page target the worker belongs to.
    target: String,
    id: String,
    url: String,
    next: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    closed: watch::Sender<bool>,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Worker")
            .field("url", &self.inner.url)
            .finish()
    }
}

impl Worker {
    pub(crate) fn new(page: &Page, target: &str, id: &str, url: &str) -> Worker {
        Worker {
            inner: Arc::new(WorkerInner {
                page: Arc::downgrade(&page.inner),
                target: target.to_owned(),
                id: id.to_owned(),
                url: url.to_owned(),
                next: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                closed: watch::channel(false).0,
            }),
        }
    }

    pub(crate) fn id(&self) -> &str {
        &self.inner.id
    }

    pub(crate) fn target(&self) -> &str {
        &self.inner.target
    }

    fn page(&self) -> Result<Page> {
        self.inner
            .page
            .upgrade()
            .map(|inner| Page { inner })
            .ok_or_else(|| Error::TargetClosed("page closed".into()))
    }

    /// Sends a protocol command to the worker; its result.
    pub(crate) async fn call(&self, method: &str, params: Value) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::TargetClosed("worker has been closed".into()));
        }
        let page = self.page()?;
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        let message = json!({"id": id, "method": method, "params": params}).to_string();
        page.inner
            .wk
            .call_target(
                &page.inner.proxy,
                &self.inner.target,
                "Worker.sendMessageToWorker",
                json!({"workerId": self.inner.id, "message": message}),
            )
            .await?;
        let answer = async {
            rx.await
                .unwrap_or_else(|_| Err(Error::TargetClosed("worker has been closed".into())))
        };
        let sleep = page.inner.services.clock.sleep(ANSWER);
        futures::pin_mut!(answer);
        match futures::future::select(answer, sleep).await {
            futures::future::Either::Left((r, _)) => r,
            futures::future::Either::Right(_) => Err(Error::Timeout {
                ms: ANSWER.as_millis() as u64,
                action: format!("waiting for the worker to answer {method}"),
            }),
        }
    }

    /// A `Worker.dispatchMessageFromWorker` message: an answer, or an event
    /// (returned for the page to handle).
    pub(crate) fn dispatch(&self, message: &str) -> Option<Value> {
        let m: Value = serde_json::from_str(message).ok()?;
        if let Some(id) = m.get("id").and_then(Value::as_u64) {
            if let Some(tx) = self.inner.pending.lock().unwrap().remove(&id) {
                let r = match m.get("error") {
                    Some(e) => Err(Error::Protocol {
                        method: String::new(),
                        code: e["code"].as_i64().unwrap_or_default(),
                        message: e["message"].as_str().unwrap_or_default().to_owned(),
                    }),
                    None => Ok(m["result"].clone()),
                };
                let _ = tx.send(r);
            }
            return None;
        }
        Some(m)
    }

    pub(crate) fn mark_closed(&self) {
        self.inner.closed.send_replace(true);
        self.inner.pending.lock().unwrap().clear();
    }

    /// The worker script's URL.
    pub fn url(&self) -> &str {
        &self.inner.url
    }

    pub fn is_closed(&self) -> bool {
        *self.inner.closed.borrow()
    }

    /// Waits until the worker is terminated (or its page goes away).
    pub async fn wait_for_close(&self) {
        let mut rx = self.inner.closed.subscribe();
        let _ = rx.wait_for(|closed| *closed).await;
    }

    /// `worker.waitForEvent(event)`: the first event `matcher` maps to a
    /// value (WebKit reports the worker's close).
    pub async fn wait_for_event<T>(
        &self,
        matcher: impl Fn(&WorkerEvent) -> Option<T>,
    ) -> Result<T> {
        self.wait_for_close().await;
        matcher(&WorkerEvent::Close)
            .ok_or_else(|| Error::TargetClosed("worker has been closed".into()))
    }

    fn wrap(source: &str, arg: Option<&Value>) -> String {
        let source = source.trim().trim_end_matches(';');
        let arg = arg
            .map(Value::to_string)
            .unwrap_or_else(|| "undefined".into());
        format!(
            "(async (__chromiumoxide_arg) => {{ const __v = (\n{source}\n); \
             return typeof __v === 'function' ? await __v(__chromiumoxide_arg) : await __v; }})({arg})"
        )
    }

    /// The JSON value of `source` (an expression, or a function called
    /// with `arg`).
    pub(crate) async fn evaluate_value(&self, source: &str, arg: Option<&Value>) -> Result<Value> {
        let expression = format!(
            "{}.then(__r => __r === undefined ? undefined : JSON.stringify(__r))",
            Self::wrap(source, arg)
        );
        let r = self
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": false}),
            )
            .await?;
        let v = Via::Worker(self.clone()).settle(r, true).await?;
        match v.get("value").and_then(Value::as_str) {
            Some(json) => serde_json::from_str(json)
                .map_err(|e| Error::Evaluation(format!("cannot read the result: {e}"))),
            None => Ok(Value::Null),
        }
    }

    /// A handle to the value of `source`.
    pub(crate) async fn evaluate_handle_value(
        &self,
        source: &str,
        arg: Option<&Value>,
    ) -> Result<JsHandle> {
        let r = self
            .call(
                "Runtime.evaluate",
                json!({"expression": Self::wrap(source, arg), "returnByValue": false}),
            )
            .await?;
        let remote = Via::Worker(self.clone()).settle(r, false).await?;
        Ok(JsHandle {
            page: self.page()?,
            frame: None,
            remote,
            worker: Some(self.clone()),
        })
    }

    /// `worker.evaluate(expression or function)`.
    pub async fn evaluate<T: DeserializeOwned>(&self, source: &str) -> Result<T> {
        let v = self.evaluate_value(source, None).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `worker.evaluate(fn, arg)`.
    pub async fn evaluate_with<T: DeserializeOwned, A: Serialize>(
        &self,
        source: &str,
        arg: A,
    ) -> Result<T> {
        let arg = serde_json::to_value(arg)?;
        let v = self.evaluate_value(source, Some(&arg)).await?;
        serde_json::from_value(v)
            .map_err(|e| Error::Evaluation(format!("cannot deserialize result: {e}")))
    }

    /// `worker.evaluateHandle(expression or function)`.
    pub async fn evaluate_handle(&self, source: &str) -> Result<JsHandle> {
        self.evaluate_handle_value(source, None).await
    }
}

impl Page {
    /// `page.workers()`: the page's running dedicated workers.
    pub fn workers(&self) -> Vec<Worker> {
        self.inner
            .workers
            .lock()
            .unwrap()
            .values()
            .filter(|w| !w.is_closed())
            .cloned()
            .collect()
    }

    /// `Worker.*` events of a page target.
    pub(crate) fn on_worker_event(&self, target: &str, method: &str, p: &Value) {
        let id = p["workerId"].as_str().unwrap_or_default().to_owned();
        match method {
            "Worker.workerCreated" => {
                let worker = Worker::new(self, target, &id, p["url"].as_str().unwrap_or_default());
                self.inner
                    .workers
                    .lock()
                    .unwrap()
                    .insert(id.clone(), worker.clone());
                let page = self.clone();
                tokio::spawn(async move {
                    // The worker waits for `Worker.initialized`; the
                    // domains are enabled first, as in Playwright.
                    let runtime = worker.call("Runtime.enable", json!({}));
                    let console = worker.call("Console.enable", json!({}));
                    let init = page.inner.wk.call_target(
                        &page.inner.proxy,
                        worker.target(),
                        "Worker.initialized",
                        json!({"workerId": worker.id()}),
                    );
                    let _ = futures::join!(runtime, console, init);
                    page.emit(crate::application::webkit_events::PageEvent::Worker(worker));
                });
            }
            "Worker.workerTerminated" => {
                if let Some(w) = self.inner.workers.lock().unwrap().remove(&id) {
                    w.mark_closed();
                }
            }
            "Worker.dispatchMessageFromWorker" => {
                let worker = self.inner.workers.lock().unwrap().get(&id).cloned();
                let Some(worker) = worker else {
                    return;
                };
                let Some(event) = worker.dispatch(p["message"].as_str().unwrap_or_default()) else {
                    return;
                };
                if event["method"] == "Console.messageAdded" {
                    self.on_console(&event["params"]["message"], Some(worker));
                }
            }
            _ => {}
        }
    }

    /// Marks the workers of a gone target closed.
    pub(crate) fn close_workers_of(&self, target: &str) {
        let mut workers = self.inner.workers.lock().unwrap();
        workers.retain(|_, w| {
            if w.target() == target {
                w.mark_closed();
                false
            } else {
                true
            }
        });
    }
}
