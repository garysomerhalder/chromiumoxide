//! WebKit history, cookies, init scripts and page events
//! (`WKPage`/`WKBrowserContext` in Playwright).

use serde_json::{Value, json};

use crate::application::webkit::{BrowserContext, Page};
use crate::application::webkit_network::{Request, Response};
use crate::domain::error::{Error, Result};
use crate::domain::storage::{Cookie, NewCookie, filter_cookies};
use crate::domain::webkit::{cookie_from_webkit, cookie_to_webkit};

/// Something that happened on a WebKit page (`page.on(...)`).
#[derive(Debug, Clone)]
pub enum PageEvent {
    Console(ConsoleMessage),
    PageError(PageError),
    Dialog(Dialog),
    DomContentLoaded,
    Load,
    Close,
    /// A request was issued (`page.on('request')`).
    Request(crate::application::webkit_network::Request),
    /// Response headers arrived (`page.on('response')`).
    Response(crate::application::webkit_network::Response),
    /// The response finished loading (`page.on('requestfinished')`).
    RequestFinished(crate::application::webkit_network::Request),
    /// The request failed (`page.on('requestfailed')`).
    RequestFailed(crate::application::webkit_network::Request),
    /// The page opened a pop-up (`page.on('popup')`).
    Popup(Page),
    /// A download started (`page.on('download')`).
    Download(crate::application::firefox_download::Download),
}

/// Something that happened in a WebKit context (`context.on(...)`).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    /// A page opened in the context (`context.on('page')`).
    Page(Page),
    Console(ConsoleMessage),
    Dialog(Dialog),
    Request(Request),
    Response(Response),
    RequestFinished(Request),
    RequestFailed(Request),
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
    pub async fn accept(&self, prompt_text: Option<&str>) -> Result<()> {
        self.page.handle_dialog(true, prompt_text).await
    }
    /// `dialog.dismiss()`.
    pub async fn dismiss(&self) -> Result<()> {
        self.page.handle_dialog(false, None).await
    }
}

/// Playwright's `splitErrorMessage`: `"Error: boom"` → (`Error`, `boom`).
fn split_error_message(text: &str) -> (String, String) {
    match text.find(':') {
        Some(i) => (
            text[..i].to_owned(),
            text.get(i + 2..).unwrap_or_default().to_owned(),
        ),
        None => (String::new(), text.to_owned()),
    }
}

/// A console argument as Playwright prints it.
fn console_arg(p: &Value) -> String {
    match p.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => p
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| p["type"].as_str().unwrap_or("undefined").to_owned()),
        Some(v) => v.to_string(),
    }
}

/// A WebKit headers object as name / value pairs.
fn header_pairs(h: &Value) -> Vec<(String, String)> {
    h.as_object()
        .map(|h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

/// A WebKit `Network.Response`.
fn response(r: &Value) -> Response {
    Response {
        url: r["url"].as_str().unwrap_or_default().to_owned(),
        status: r["status"].as_u64().unwrap_or_default() as u16,
        status_text: r["statusText"].as_str().unwrap_or_default().to_owned(),
        headers: header_pairs(&r["headers"]),
    }
}

/// `WKBrowserContext._calculateBootstrapScript`'s fixed part.
const PUBLIC_KEY_CREDENTIAL: &str = "(function polyfill() {\n          window.PublicKeyCredential ??= {\n            async getClientCapabilities() {\n              return {};\n            },\n            async isConditionalMediationAvailable() {\n              return false;\n            },\n            async isUserVerifyingPlatformAuthenticatorAvailable() {\n              return false;\n            }\n          };\n        })();";

impl Page {
    /// The page's bootstrap script (`WKPage._calculateBootstrapScript`):
    /// Playwright's shims, then the context's and the page's init scripts.
    pub(crate) fn bootstrap_script(&self) -> String {
        let mut scripts: Vec<String> = Vec::new();
        if !self.inner.options.is_mobile {
            scripts.push("delete window.orientation".into());
        }
        scripts.push(
            "if (!window.safari) window.safari = { pushNotification: { toString() { return \"[object SafariRemoteNotification]\"; } } };"
                .into(),
        );
        scripts.push(
            "if (!window.GestureEvent) window.GestureEvent = function GestureEvent() {};".into(),
        );
        scripts.push(PUBLIC_KEY_CREDENTIAL.into());
        if let Some(context) = self.inner.context.upgrade() {
            scripts.extend(context.init_scripts.lock().unwrap().iter().cloned());
        }
        scripts.extend(self.inner.init_scripts.lock().unwrap().iter().cloned());
        scripts.join(";\n")
    }

    async fn update_bootstrap_script(&self) -> Result<()> {
        self.target_call(
            "Page.setBootstrapScript",
            json!({"source": self.bootstrap_script()}),
        )
        .await
        .map(drop)
    }

    /// Sends an event to the live listeners; false when there are none.
    pub(crate) fn emit(&self, event: PageEvent) -> bool {
        self.inner.history.lock().unwrap().record(&event);
        let mut heard = false;
        if let Some(ctx) = self.inner.context.upgrade()
            && let Some(e) = ContextEvent::from_page(&event)
        {
            heard = ctx.emit(e);
        }
        let mut subs = self.inner.subscribers.lock().unwrap();
        subs.retain(|s| s.unbounded_send(event.clone()).is_ok());
        heard || !subs.is_empty()
    }

    /// `Console.messageAdded` (`WKPage._onConsoleMessage`).
    pub(crate) fn on_console(&self, m: &Value) {
        let text = m["text"].as_str().unwrap_or_default();
        if m["level"] == "error" && m["source"] == "javascript" {
            let (name, message) = split_error_message(text);
            let stack = match m["stackTrace"]["callFrames"].as_array() {
                Some(frames) => {
                    let mut s = text.to_owned();
                    for f in frames {
                        let name = f["functionName"].as_str().filter(|n| !n.is_empty());
                        s.push_str(&format!(
                            "\n    at {} ({}:{}:{})",
                            name.unwrap_or("unknown"),
                            f["url"].as_str().unwrap_or_default(),
                            f["lineNumber"],
                            f["columnNumber"]
                        ));
                    }
                    s
                }
                None => String::new(),
            };
            *self.inner.last_console.lock().unwrap() = None;
            self.emit(PageEvent::PageError(PageError {
                name,
                message,
                stack,
            }));
            return;
        }
        let kind = match (m["type"].as_str().unwrap_or_default(), m["level"].as_str()) {
            ("log", Some(level)) => level.to_owned(),
            ("timing", _) => "timeEnd".to_owned(),
            (t, _) => t.to_owned(),
        };
        let text = match m["parameters"].as_array() {
            Some(params) if !params.is_empty() => {
                params.iter().map(console_arg).collect::<Vec<_>>().join(" ")
            }
            _ => text.to_owned(),
        };
        let message = ConsoleMessage { kind, text };
        *self.inner.last_console.lock().unwrap() = Some((message.clone(), 1));
        self.emit(PageEvent::Console(message));
    }

    /// WebKit `Network.*` events as request, response, requestfinished and
    /// requestfailed (`WKPage._onRequestWillBeSent` and friends).
    pub(crate) fn on_network_event(&self, method: &str, p: &Value) {
        let id = p["requestId"].as_str().unwrap_or_default().to_owned();
        match method {
            "Network.requestWillBeSent" => {
                let r = &p["request"];
                let url = r["url"].as_str().unwrap_or_default();
                if url.starts_with("data:") || url.starts_with("about:") {
                    return;
                }
                let mut requests = self.inner.requests.lock().unwrap();
                // A redirect ends the previous request with its response.
                if let Some(prev) = requests.remove(&id)
                    && p["redirectResponse"].is_object()
                {
                    self.emit(PageEvent::Response(response(&p["redirectResponse"])));
                    self.emit(PageEvent::RequestFinished(prev));
                }
                let request = Request {
                    url: url.to_owned(),
                    method: r["method"].as_str().unwrap_or("GET").to_owned(),
                    headers: header_pairs(&r["headers"]),
                    failure: None,
                };
                requests.insert(id, request.clone());
                drop(requests);
                self.emit(PageEvent::Request(request));
            }
            "Network.responseReceived" => {
                if self.inner.requests.lock().unwrap().contains_key(&id) {
                    self.emit(PageEvent::Response(response(&p["response"])));
                }
            }
            "Network.loadingFinished" => {
                let done = self.inner.requests.lock().unwrap().remove(&id);
                if let Some(r) = done {
                    self.emit(PageEvent::RequestFinished(r));
                }
            }
            "Network.loadingFailed" => {
                let failed = self.inner.requests.lock().unwrap().remove(&id);
                if let Some(mut r) = failed {
                    r.failure = Some(p["errorText"].as_str().unwrap_or("failed").to_owned());
                    self.emit(PageEvent::RequestFailed(r));
                }
            }
            _ => {}
        }
    }

    /// `Dialog.javascriptDialogOpening`: to the listeners, else dismissed.
    pub(crate) fn on_dialog(&self, p: &Value) {
        let s = |k: &str| p[k].as_str().unwrap_or_default().to_owned();
        let dialog = Dialog {
            page: self.clone(),
            kind: s("type"),
            message: s("message"),
            default_value: s("defaultPrompt"),
        };
        if !self.emit(PageEvent::Dialog(dialog)) {
            let page = self.clone();
            tokio::spawn(async move { page.handle_dialog(false, None).await });
        }
    }

    async fn handle_dialog(&self, accept: bool, prompt_text: Option<&str>) -> Result<()> {
        let mut params = json!({"accept": accept});
        if let Some(t) = prompt_text {
            params["promptText"] = json!(t);
        }
        self.proxy_call("Dialog.handleJavaScriptDialog", params)
            .await
            .map(drop)
    }

    /// Runs a history command on the page target and waits for the load.
    async fn history(&self, method: &str) -> Result<()> {
        self.check_open()?;
        let mut loads = self.inner.loads.subscribe();
        let before = *loads.borrow_and_update();
        *self.inner.load_error.lock().unwrap() = None;
        self.target_call(method, json!({}))
            .await
            .map_err(|e| match e {
                Error::Protocol { message, .. } => Error::Navigation(message),
                other => other,
            })?;
        self.wait_for_load(loads, before).await
    }

    /// `page.reload()`.
    pub async fn reload(&self) -> Result<()> {
        self.history("Page.reload").await
    }
    /// `page.goBack()`.
    pub async fn go_back(&self) -> Result<()> {
        self.history("Page.goBack").await
    }
    /// `page.goForward()`.
    pub async fn go_forward(&self) -> Result<()> {
        self.history("Page.goForward").await
    }
    /// `page.addInitScript(source)`.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.inner
            .init_scripts
            .lock()
            .unwrap()
            .push(source.to_owned());
        self.update_bootstrap_script().await
    }
    /// `page.events()`: console messages, errors, dialogs, load and close
    /// from now on. While nobody listens, dialogs are dismissed.
    pub fn events(&self) -> impl futures::Stream<Item = PageEvent> + Send + use<> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }
}

impl BrowserContext {
    /// The context's open pages.
    pub(crate) fn live_pages(&self) -> Vec<Page> {
        let mut pages = self.inner.pages.lock().unwrap();
        pages.retain(|p| p.strong_count() > 0);
        pages
            .iter()
            .filter_map(|p| p.upgrade())
            .map(|inner| Page { inner })
            .collect()
    }

    /// `context.cookies()`.
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        self.cookies_for(&[]).await
    }
    /// `context.cookies(urls)`.
    pub async fn cookies_for(&self, urls: &[&str]) -> Result<Vec<Cookie>> {
        let r = self
            .wk()
            .call(
                None,
                "Playwright.getAllCookies",
                json!({"browserContextId": self.inner.id}),
            )
            .await?;
        let cookies = r["cookies"]
            .as_array()
            .map(|a| a.iter().map(cookie_from_webkit).collect())
            .unwrap_or_default();
        Ok(filter_cookies(cookies, urls))
    }
    /// `context.addCookies(cookies)`.
    pub async fn add_cookies(&self, cookies: &[NewCookie]) -> Result<()> {
        let resolved = cookies
            .iter()
            .map(NewCookie::resolve)
            .collect::<std::result::Result<Vec<_>, String>>()
            .map_err(Error::InvalidArgument)?;
        let cookies: Vec<Value> = resolved.iter().map(cookie_to_webkit).collect();
        self.wk()
            .call(
                None,
                "Playwright.setCookies",
                json!({"cookies": cookies, "browserContextId": self.inner.id}),
            )
            .await
            .map(drop)
    }
    /// `context.clearCookies()`.
    pub async fn clear_cookies(&self) -> Result<()> {
        self.wk()
            .call(
                None,
                "Playwright.deleteAllCookies",
                json!({"browserContextId": self.inner.id}),
            )
            .await
            .map(drop)
    }
    /// `context.addInitScript(source)`: for its pages from now on, and
    /// their next documents.
    pub async fn add_init_script(&self, source: &str) -> Result<()> {
        self.inner
            .init_scripts
            .lock()
            .unwrap()
            .push(source.to_owned());
        for page in self.live_pages() {
            page.update_bootstrap_script().await?;
        }
        Ok(())
    }
}
