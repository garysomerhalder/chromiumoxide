//! Page events on the Firefox engine (`page.events()`): console messages,
//! uncaught errors, dialogs, load and close, from BiDi `log` and
//! `browsingContext` events.

use serde_json::json;

use crate::application::firefox::Page;
use crate::domain::error::Result;

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
}

/// `page.on('console')`.
#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    pub(crate) kind: String,
    pub(crate) text: String,
}

impl ConsoleMessage {
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
