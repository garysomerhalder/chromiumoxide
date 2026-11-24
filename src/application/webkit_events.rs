//! WebKit history, cookies, init scripts and page events
//! (`WKPage`/`WKBrowserContext` in Playwright).

use crate::application::webkit::{BrowserContext, Page};
use crate::domain::error::{Error, Result};
use crate::domain::storage::{Cookie, NewCookie};

/// Something that happened on a WebKit page (`page.on(...)`).
#[derive(Debug, Clone)]
pub enum PageEvent {
    Console(ConsoleMessage),
    PageError(PageError),
    Dialog(Dialog),
    DomContentLoaded,
    Load,
    Close,
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
    pub async fn accept(&self, _prompt_text: Option<&str>) -> Result<()> {
        let _ = &self.page;
        Err(todo())
    }
    /// `dialog.dismiss()`.
    pub async fn dismiss(&self) -> Result<()> {
        Err(todo())
    }
}

fn todo() -> Error {
    Error::InvalidArgument("not implemented on WebKit yet".into())
}

impl Page {
    /// `page.reload()`.
    pub async fn reload(&self) -> Result<()> {
        Err(todo())
    }
    /// `page.goBack()`.
    pub async fn go_back(&self) -> Result<()> {
        Err(todo())
    }
    /// `page.goForward()`.
    pub async fn go_forward(&self) -> Result<()> {
        Err(todo())
    }
    /// `page.addInitScript(source)`.
    pub async fn add_init_script(&self, _source: &str) -> Result<()> {
        Err(todo())
    }
    /// `page.events()`.
    pub fn events(&self) -> impl futures::Stream<Item = PageEvent> + Send + use<> {
        futures::stream::empty()
    }
}

impl BrowserContext {
    /// `context.cookies()`.
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        Err(todo())
    }
    /// `context.cookies(urls)`.
    pub async fn cookies_for(&self, _urls: &[&str]) -> Result<Vec<Cookie>> {
        Err(todo())
    }
    /// `context.addCookies(cookies)`.
    pub async fn add_cookies(&self, _cookies: &[NewCookie]) -> Result<()> {
        Err(todo())
    }
    /// `context.clearCookies()`.
    pub async fn clear_cookies(&self) -> Result<()> {
        Err(todo())
    }
    /// `context.addInitScript(source)`.
    pub async fn add_init_script(&self, _source: &str) -> Result<()> {
        Err(todo())
    }
}
