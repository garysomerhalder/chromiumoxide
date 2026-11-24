//! Page events on the Firefox engine (`page.events()`): console messages,
//! uncaught errors, dialogs, load and close, from BiDi `log` and
//! `browsingContext` events.

use serde_json::json;

use crate::application::firefox::Page;
use crate::domain::error::Result;

/// `context.on(...)` events of a Firefox context (`context.events()`).
#[derive(Clone, Debug)]
pub enum ContextEvent {
    /// A page opened in the context (`context.on('page')`).
    Page(crate::application::firefox::Page),
    Console(ConsoleMessage),
    Dialog(Dialog),
    Request(crate::application::firefox_route::Request),
    Response(crate::application::firefox_route::Response),
    RequestFinished(crate::application::firefox_route::Request),
    RequestFailed(crate::application::firefox_route::Request),
    /// An uncaught error in a page (`context.on('weberror')`).
    WebError(PageError),
    Close,
}

impl ContextEvent {
    /// The context event a page event is also reported as, if any.
    pub(crate) fn from_page(event: &PageEvent) -> Option<ContextEvent> {
        Some(match event {
            PageEvent::Console(m) => ContextEvent::Console(m.clone()),
            PageEvent::Dialog(d) => ContextEvent::Dialog(d.clone()),
            PageEvent::Request(r) => ContextEvent::Request(r.clone()),
            PageEvent::Response(r) => ContextEvent::Response(r.clone()),
            PageEvent::RequestFinished(r) => ContextEvent::RequestFinished(r.clone()),
            PageEvent::RequestFailed(r) => ContextEvent::RequestFailed(r.clone()),
            PageEvent::PageError(e) => ContextEvent::WebError(e.clone()),
            _ => return None,
        })
    }
}

/// Something that happened on a Firefox page (`page.on(...)` in Playwright).
#[derive(Debug, Clone)]
pub enum PageEvent {
    Console(ConsoleMessage),
    PageError(PageError),
    Dialog(Dialog),
    DomContentLoaded,
    Load,
    Close,
    /// A request was issued (`page.on('request')`).
    Request(crate::application::firefox_route::Request),
    /// Response headers arrived (`page.on('response')`).
    Response(crate::application::firefox_route::Response),
    /// The response finished loading (`page.on('requestfinished')`).
    RequestFinished(crate::application::firefox_route::Request),
    /// The request failed (`page.on('requestfailed')`).
    RequestFailed(crate::application::firefox_route::Request),
    /// A download started (`page.on('download')`).
    Download(crate::application::firefox_download::Download),
    /// The page opened a pop-up (`page.on('popup')`).
    Popup(crate::application::firefox::Page),
    /// A dedicated worker started (`page.on('worker')`).
    Worker(crate::application::firefox_worker::Worker),
    /// The page opened a WebSocket (`page.on('websocket')`).
    WebSocket(crate::application::websocket::WebSocket),
    /// A file input asked for files (`page.on('filechooser')`).
    FileChooser(FileChooser),
}

/// `page.on('filechooser')`: a file input's chooser (BiDi
/// `input.fileDialogOpened`); answer it with [`FileChooser::set_files`].
#[derive(Clone, Debug)]
pub struct FileChooser {
    pub(crate) page: Page,
    pub(crate) element: crate::application::firefox_handle::ElementHandle,
    pub(crate) multiple: bool,
}

impl FileChooser {
    /// `fileChooser.page()`.
    pub fn page(&self) -> &Page {
        &self.page
    }
    /// `fileChooser.element()`: the `<input type=file>`.
    pub fn element(&self) -> crate::application::firefox_handle::ElementHandle {
        self.element.clone()
    }
    /// `fileChooser.isMultiple()`.
    pub fn is_multiple(&self) -> bool {
        self.multiple
    }
    /// `fileChooser.setFiles(files)`.
    pub async fn set_files(&self, files: &[std::path::PathBuf]) -> Result<()> {
        self.element.set_input_files(files).await
    }
}

/// `page.on('console')`.
#[derive(Clone)]
pub struct ConsoleMessage {
    pub(crate) kind: String,
    pub(crate) text: String,
    /// BiDi's `RemoteValue`s of the arguments.
    pub(crate) args: Vec<serde_json::Value>,
    pub(crate) timestamp: f64,
    pub(crate) location: crate::domain::console::Location,
    /// The browsing context that logged it.
    pub(crate) context: String,
    pub(crate) page: std::sync::Weak<crate::application::firefox::PageInner>,
}

impl std::fmt::Debug for ConsoleMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::ConsoleMessage")
            .field("kind", &self.kind)
            .field("text", &self.text)
            .finish()
    }
}

impl ConsoleMessage {
    /// `consoleMessage.args()`: handles to the logged values.
    pub fn args(&self) -> Vec<crate::application::firefox_handle::JsHandle> {
        let Some(page) = self.page() else {
            return Vec::new();
        };
        self.args
            .iter()
            .map(|remote| crate::application::firefox_handle::JsHandle {
                page: page.clone(),
                target: json!({"context": self.context}),
                remote: remote.clone(),
            })
            .collect()
    }
    /// `consoleMessage.location()`: where it was logged.
    pub fn location(&self) -> &crate::domain::console::Location {
        &self.location
    }
    /// `consoleMessage.timestamp()`: milliseconds since the epoch.
    pub fn timestamp(&self) -> f64 {
        self.timestamp
    }
    /// `consoleMessage.page()`.
    pub fn page(&self) -> Option<Page> {
        self.page.upgrade().map(|inner| Page { inner })
    }
    /// `consoleMessage.worker()`: worker messages are not reported on
    /// Firefox (BiDi cannot reach worker realms), so always `None`.
    pub fn worker(&self) -> Option<crate::application::firefox_worker::Worker> {
        None
    }
    /// `log`, `warning`, `error`, `info`, `debug`, ... (Playwright's names).
    pub fn kind(&self) -> &str {
        &self.kind
    }
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// `page.on('pageerror')`: an uncaught exception.
#[derive(Debug, Clone)]
pub struct PageError {
    pub(crate) name: String,
    pub(crate) message: String,
    pub(crate) stack: String,
}

impl PageError {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub fn stack(&self) -> &str {
        &self.stack
    }

    /// From BiDi's `"Error: boom"` text.
    pub(crate) fn from_text(text: &str, stack: String) -> Self {
        let (name, message) = match text.split_once(": ") {
            Some((n, m)) if !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || c == '_') => {
                (n.to_owned(), m.to_owned())
            }
            _ => (String::new(), text.to_owned()),
        };
        PageError {
            name,
            message,
            stack,
        }
    }
}

/// `page.on('dialog')`: alert, confirm, prompt or beforeunload.
#[derive(Debug, Clone)]
pub struct Dialog {
    pub(crate) page: Page,
    pub(crate) kind: String,
    pub(crate) message: String,
    pub(crate) default_value: String,
}

impl Dialog {
    /// `dialog.page()`.
    pub fn page(&self) -> &Page {
        &self.page
    }
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

    /// `dialog.accept(promptText?)`.
    pub async fn accept(&self, prompt_text: Option<&str>) -> Result<()> {
        self.page.handle_user_prompt(true, prompt_text).await
    }

    /// `dialog.dismiss()`.
    pub async fn dismiss(&self) -> Result<()> {
        self.page.handle_user_prompt(false, None).await
    }
}

/// Playwright's console type names (`warn` is `warning`).
pub(crate) fn console_kind(method: &str) -> String {
    match method {
        "warn" => "warning".to_owned(),
        other => other.to_owned(),
    }
}

/// `browsingContext.handleUserPrompt` parameters.
pub(crate) fn prompt_params(context: &str, accept: bool, text: Option<&str>) -> serde_json::Value {
    let mut p = json!({"context": context, "accept": accept});
    if let Some(t) = text {
        p["userText"] = json!(t);
    }
    p
}
