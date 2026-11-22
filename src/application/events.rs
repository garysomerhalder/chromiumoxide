//! Page events (Playwright's `page.on(...)`), event waiters, console and
//! error history, and dialogs.

use std::collections::VecDeque;
use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::{FutureExt, Stream, StreamExt};
use serde_json::{Value, json};

use crate::application::handle::JsHandle;
use crate::application::page::Page;
use crate::domain::console::{Location, PageError, console_text, exception_to_error, location};
use crate::domain::error::{Error, Result};
use crate::ports::RawEvent;

/// How many console messages / page errors a page remembers.
const HISTORY: usize = 200;

/// Something that happened on a page.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum PageEvent {
    Console(ConsoleMessage),
    PageError(PageError),
    Dialog(Dialog),
    DomContentLoaded,
    Load,
    Close,
}

/// A `console.*` call (Playwright's `ConsoleMessage`).
#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    kind: String,
    text: String,
    args: Vec<JsHandle>,
    location: Location,
}

impl ConsoleMessage {
    /// `log`, `warning`, `error`, `info`, `debug`, `table`, ...
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn args(&self) -> &[JsHandle] {
        &self.args
    }
    pub fn location(&self) -> &Location {
        &self.location
    }
}

type DialogHandler = Arc<dyn Fn(Dialog) -> BoxFuture<'static, ()> + Send + Sync>;

/// A JavaScript dialog (alert, confirm, prompt, beforeunload).
#[derive(Clone)]
pub struct Dialog {
    page: Page,
    kind: String,
    message: String,
    default_value: String,
    handled: Arc<AtomicBool>,
}

impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish()
    }
}

impl Dialog {
    /// `alert`, `confirm`, `prompt` or `beforeunload`.
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn default_value(&self) -> &str {
        &self.default_value
    }
    pub fn page(&self) -> &Page {
        &self.page
    }

    async fn handle(&self, accept: bool, prompt: Option<&str>) -> Result<()> {
        if self.handled.swap(true, Ordering::SeqCst) {
            return Err(Error::InvalidArgument(
                "Cannot accept dialog which is already handled!".into(),
            ));
        }
        let mut params = json!({"accept": accept});
        if let Some(text) = prompt {
            params["promptText"] = json!(text);
        }
        self.page
            .send("Page.handleJavaScriptDialog", params)
            .await?;
        Ok(())
    }

    /// `dialog.accept(promptText?)`.
    pub async fn accept(&self, prompt_text: Option<&str>) -> Result<()> {
        self.handle(true, prompt_text).await
    }

    /// `dialog.dismiss()`.
    pub async fn dismiss(&self) -> Result<()> {
        self.handle(false, None).await
    }
}

/// Per-page event state.
#[derive(Default)]
pub(crate) struct EventState {
    subscribers: Vec<mpsc::UnboundedSender<PageEvent>>,
    console: VecDeque<ConsoleMessage>,
    errors: VecDeque<PageError>,
    dialog_handlers: Vec<DialogHandler>,
}

impl Page {
    /// Every event from now on (Playwright's `page.on(...)` listeners).
    pub fn events(&self) -> impl Stream<Item = PageEvent> + Send + use<> {
        let (tx, rx) = mpsc::unbounded();
        self.inner.events.lock().unwrap().subscribers.push(tx);
        rx
    }

    pub(crate) fn emit(&self, event: PageEvent) {
        let mut st = self.inner.events.lock().unwrap();
        match &event {
            PageEvent::Console(m) => {
                if st.console.len() == HISTORY {
                    st.console.pop_front();
                }
                st.console.push_back(m.clone());
            }
            PageEvent::PageError(e) => {
                if st.errors.len() == HISTORY {
                    st.errors.pop_front();
                }
                st.errors.push_back(e.clone());
            }
            _ => {}
        }
        st.subscribers
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }

    /// `page.consoleMessages()`: recent console messages.
    pub fn console_messages(&self) -> Vec<ConsoleMessage> {
        self.inner
            .events
            .lock()
            .unwrap()
            .console
            .iter()
            .cloned()
            .collect()
    }

    /// `page.pageErrors()`: recent uncaught exceptions.
    pub fn page_errors(&self) -> Vec<PageError> {
        self.inner
            .events
            .lock()
            .unwrap()
            .errors
            .iter()
            .cloned()
            .collect()
    }

    /// `page.on('dialog', handler)`. Without any handler, dialogs are
    /// dismissed automatically (beforeunload is accepted), like Playwright.
    pub fn on_dialog<F, Fut>(&self, handler: F)
    where
        F: Fn(Dialog) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler: DialogHandler = Arc::new(move |d| handler(d).boxed());
        self.inner
            .events
            .lock()
            .unwrap()
            .dialog_handlers
            .push(handler);
    }

    /// Handles page-level protocol events; true if consumed.
    pub(crate) fn handle_page_event(&self, e: &RawEvent) -> bool {
        let p = &e.params;
        match e.method.as_str() {
            "Runtime.consoleAPICalled" => {
                if p.get("executionContextId").and_then(Value::as_i64) == Some(0) {
                    return true;
                }
                let args: Vec<Value> = p
                    .get("args")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let message = ConsoleMessage {
                    kind: p
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("log")
                        .to_owned(),
                    text: console_text(&args),
                    args: args
                        .iter()
                        .map(|a| JsHandle::from_remote(self, a))
                        .collect(),
                    location: location(p.get("stackTrace")),
                };
                self.emit(PageEvent::Console(message));
                true
            }
            "Runtime.exceptionThrown" => {
                let error = exception_to_error(&p["exceptionDetails"]);
                self.emit(PageEvent::PageError(error));
                true
            }
            "Page.javascriptDialogOpening" => {
                let dialog = Dialog {
                    page: self.clone(),
                    kind: p
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("alert")
                        .to_owned(),
                    message: p
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    default_value: p
                        .get("defaultPrompt")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    handled: Arc::new(AtomicBool::new(false)),
                };
                let handlers = self.inner.events.lock().unwrap().dialog_handlers.clone();
                self.emit(PageEvent::Dialog(dialog.clone()));
                if handlers.is_empty() {
                    let accept = dialog.kind == "beforeunload";
                    tokio::spawn(async move {
                        let _ = dialog.handle(accept, None).await;
                    });
                } else {
                    for h in handlers {
                        tokio::spawn(h(dialog.clone()));
                    }
                }
                true
            }
            "Page.domContentEventFired" => {
                self.emit(PageEvent::DomContentLoaded);
                true
            }
            "Page.loadEventFired" => {
                self.emit(PageEvent::Load);
                true
            }
            _ => false,
        }
    }

    /// `page.waitForEvent(...)` with a predicate that extracts a value.
    pub fn wait_for_event<T, F>(&self, matcher: F) -> EventWaiter<'_, T>
    where
        F: Fn(&PageEvent) -> Option<T> + Send + Sync + 'static,
        T: Send + 'static,
    {
        EventWaiter {
            page: self,
            matcher: Box::new(matcher),
            timeout: None,
        }
    }

    /// `page.waitForEvent('console', predicate)`.
    pub fn wait_for_console_message<F>(&self, predicate: F) -> EventWaiter<'_, ConsoleMessage>
    where
        F: Fn(&ConsoleMessage) -> bool + Send + Sync + 'static,
    {
        self.wait_for_event(move |e| match e {
            PageEvent::Console(m) if predicate(m) => Some(m.clone()),
            _ => None,
        })
    }

    /// `page.waitForEvent('pageerror')`.
    pub fn wait_for_page_error(&self) -> EventWaiter<'_, PageError> {
        self.wait_for_event(|e| match e {
            PageEvent::PageError(err) => Some(err.clone()),
            _ => None,
        })
    }
}

type Matcher<T> = Box<dyn Fn(&PageEvent) -> Option<T> + Send + Sync>;

/// A pending `waitForEvent`: `.await` it, or `.run(action)` to start waiting
/// before the action that triggers the event (no race).
#[must_use = "does nothing until awaited"]
pub struct EventWaiter<'a, T> {
    page: &'a Page,
    matcher: Matcher<T>,
    timeout: Option<Duration>,
}

impl<'a, T: Send + 'static> EventWaiter<'a, T> {
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Subscribes, runs `action`, then waits for the event.
    pub async fn run<A, R>(self, action: A) -> Result<T>
    where
        A: IntoFuture<Output = Result<R>>,
    {
        let events = self.page.events();
        action.await?;
        self.wait_on(events).await
    }

    async fn wait_on(self, mut events: impl Stream<Item = PageEvent> + Unpin) -> Result<T> {
        let timeout = self.timeout.unwrap_or_else(|| self.page.default_timeout());
        let matcher = self.matcher;
        let find = async move {
            while let Some(e) = events.next().await {
                if let Some(v) = matcher(&e) {
                    return Ok(v);
                }
                if matches!(e, PageEvent::Close) {
                    return Err(Error::TargetClosed(
                        "page closed while waiting for an event".into(),
                    ));
                }
            }
            Err(Error::TargetClosed(
                "page closed while waiting for an event".into(),
            ))
        };
        crate::application::wait::with_timeout(
            &*self.page.inner.shared.services.clock,
            timeout,
            "waiting for event",
            find,
        )
        .await
    }
}

impl<'a, T: Send + 'static> IntoFuture for EventWaiter<'a, T> {
    type Output = Result<T>;
    type IntoFuture = BoxFuture<'a, Result<T>>;
    fn into_future(self) -> Self::IntoFuture {
        async move {
            let events = self.page.events();
            self.wait_on(events).await
        }
        .boxed()
    }
}
