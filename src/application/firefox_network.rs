//! Requests and responses on the Firefox engine: Playwright's `Request`
//! and `Response`, kept up to date by BiDi `network` events (response,
//! completion or failure, redirects, timing, sizes). Bodies come from the
//! context's BiDi data collector.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::firefox::{Frame, Page, PageInner};
use crate::application::firefox_worker::Worker;
use crate::application::network::{SecurityDetails, ServerAddr};
use crate::domain::error::{Error, Result};
use crate::domain::firefox::{http_version_of, resource_type, timing_from_bidi};
use crate::domain::network::{RequestSizes, Timing};

/// A network request (`PageEvent::Request` and friends, or a route's).
#[derive(Clone)]
pub struct Request {
    pub(crate) inner: Arc<RequestInner>,
}

pub(crate) struct RequestInner {
    /// BiDi's request id (shared by the hops of a redirect chain).
    pub(crate) id: String,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    resource_type: &'static str,
    navigation: bool,
    /// The browsing context (frame) that made the request.
    context: String,
    headers_size: u64,
    body_size: u64,
    /// The request body, read from the data collector when it was sent.
    post_data: Option<Vec<u8>>,
    redirected_from: Option<Request>,
    page: Weak<PageInner>,
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}

#[derive(Default)]
struct State {
    response: Option<Arc<ResponseData>>,
    redirected_to: Option<Request>,
    finished: bool,
    failure: Option<String>,
    timing: Option<Timing>,
}

/// A response's status line, headers and sizes.
#[derive(Debug)]
pub(crate) struct ResponseData {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    protocol: String,
    headers_size: u64,
    body_size: u64,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Request")
            .field("url", &self.inner.url)
            .field("method", &self.inner.method)
            .finish()
    }
}

/// BiDi `network.Header`s as name / value pairs.
pub(crate) fn bidi_headers(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|hs| {
            hs.iter()
                .map(|h| {
                    (
                        h["name"].as_str().unwrap_or_default().to_owned(),
                        h.pointer("/value/value")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Playwright's `allHeaders()`: lower-case names, repeated headers joined
/// with `, ` (`set-cookie` with new lines).
fn all_headers(headers: &[(String, String)]) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for (k, v) in headers {
        let k = k.to_ascii_lowercase();
        let sep = if k == "set-cookie" { "\n" } else { ", " };
        out.entry(k)
            .and_modify(|e| {
                e.push_str(sep);
                e.push_str(v);
            })
            .or_insert_with(|| v.clone());
    }
    out
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

impl Request {
    /// From a BiDi `network.beforeRequestSent` (or another `network`
    /// event's) parameters.
    pub(crate) fn from_event(
        p: &Value,
        page: Weak<PageInner>,
        redirected_from: Option<Request>,
        post_data: Option<Vec<u8>>,
    ) -> Request {
        let r = &p["request"];
        Request {
            inner: Arc::new(RequestInner {
                id: r["request"].as_str().unwrap_or_default().to_owned(),
                url: r["url"].as_str().unwrap_or_default().to_owned(),
                method: r["method"].as_str().unwrap_or("GET").to_owned(),
                headers: bidi_headers(&r["headers"]),
                resource_type: resource_type(
                    r["destination"].as_str().unwrap_or_default(),
                    r["initiatorType"].as_str(),
                ),
                navigation: p["navigation"].is_string(),
                context: p["context"].as_str().unwrap_or_default().to_owned(),
                headers_size: r["headersSize"].as_u64().unwrap_or_default(),
                body_size: r["bodySize"].as_u64().unwrap_or_default(),
                post_data,
                redirected_from,
                page,
                state: Mutex::new(State::default()),
                changed: tokio::sync::Notify::new(),
            }),
        }
    }

    fn update(&self, f: impl FnOnce(&mut State)) {
        f(&mut self.inner.state.lock().unwrap());
        self.inner.changed.notify_waiters();
    }

    /// A response arrived (`network.responseStarted`); its `Response`.
    pub(crate) fn responded(&self, p: &Value) -> Response {
        let r = &p["response"];
        let data = Arc::new(ResponseData {
            url: r["url"]
                .as_str()
                .filter(|u| !u.is_empty())
                .unwrap_or(&self.inner.url)
                .to_owned(),
            status: r["status"].as_u64().unwrap_or_default() as u16,
            status_text: r["statusText"].as_str().unwrap_or_default().to_owned(),
            headers: bidi_headers(&r["headers"]),
            protocol: r["protocol"].as_str().unwrap_or_default().to_owned(),
            headers_size: r["headersSize"].as_u64().unwrap_or_default(),
            body_size: r["bodySize"].as_u64().unwrap_or_default(),
        });
        self.update(|s| s.response = Some(data.clone()));
        Response {
            data,
            request: self.clone(),
        }
    }

    /// The response completed (`network.responseCompleted`).
    pub(crate) fn completed(&self, p: &Value) {
        if self.inner.state.lock().unwrap().response.is_none() {
            self.responded(p);
        }
        let timing = timing_from_bidi(&p["request"]["timings"]);
        let sizes = &p["response"];
        self.update(|s| {
            if let Some(old) = &s.response {
                s.response = Some(Arc::new(ResponseData {
                    url: old.url.clone(),
                    status: old.status,
                    status_text: old.status_text.clone(),
                    headers: old.headers.clone(),
                    protocol: old.protocol.clone(),
                    headers_size: sizes["headersSize"].as_u64().unwrap_or(old.headers_size),
                    body_size: sizes["bodySize"].as_u64().unwrap_or(old.body_size),
                }));
            }
            s.timing = Some(timing);
            s.finished = true;
        });
    }

    /// The request failed (`network.fetchError`).
    pub(crate) fn failed(&self, error: &str) {
        self.update(|s| {
            s.failure = Some(error.to_owned());
            s.finished = true;
        });
    }

    /// The next hop of a redirect.
    pub(crate) fn redirected(&self, to: &Request) {
        self.update(|s| {
            s.redirected_to = Some(to.clone());
            s.finished = true;
        });
    }

    pub fn url(&self) -> &str {
        &self.inner.url
    }
    pub fn method(&self) -> &str {
        &self.inner.method
    }
    /// Request headers, in order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.inner.headers
    }
    /// `request.headersArray()`.
    pub fn headers_array(&self) -> Vec<(String, String)> {
        self.inner.headers.clone()
    }
    /// `request.allHeaders()`.
    pub async fn all_headers(&self) -> Result<HashMap<String, String>> {
        Ok(all_headers(&self.inner.headers))
    }
    /// The first header with this name (case-insensitive).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        header_value(&self.inner.headers, name)
    }
    /// `request.failure()`: the error text of a failed request.
    pub fn failure(&self) -> Option<String> {
        self.inner.state.lock().unwrap().failure.clone()
    }
    /// `request.isNavigationRequest()`.
    pub fn is_navigation_request(&self) -> bool {
        self.inner.navigation
    }
    /// `request.resourceType()`: `document`, `fetch`, `xhr`, `image`, ...
    pub fn resource_type(&self) -> &str {
        self.inner.resource_type
    }
    /// `request.postData()`.
    pub async fn post_data(&self) -> Result<Option<String>> {
        Ok(self
            .inner
            .post_data
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned()))
    }
    /// `request.postDataBuffer()`.
    pub async fn post_data_buffer(&self) -> Result<Option<Vec<u8>>> {
        Ok(self.inner.post_data.clone())
    }
    /// `request.postDataJSON()`.
    pub async fn post_data_json<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        match &self.inner.post_data {
            Some(b) => Ok(Some(serde_json::from_slice(b)?)),
            None => Ok(None),
        }
    }
    /// `request.redirectedFrom()`.
    pub fn redirected_from(&self) -> Option<Request> {
        self.inner.redirected_from.clone()
    }
    /// `request.redirectedTo()`.
    pub fn redirected_to(&self) -> Option<Request> {
        self.inner.state.lock().unwrap().redirected_to.clone()
    }
    /// `request.existingResponse()`: the response, if it arrived.
    pub fn existing_response(&self) -> Option<Response> {
        let data = self.inner.state.lock().unwrap().response.clone()?;
        Some(Response {
            data,
            request: self.clone(),
        })
    }
    /// `request.response()`: waits for the response (`None` if the
    /// request failed).
    pub async fn response(&self) -> Result<Option<Response>> {
        loop {
            let notified = self.inner.changed.notified();
            {
                let s = self.inner.state.lock().unwrap();
                if s.response.is_some() || s.finished {
                    break;
                }
            }
            notified.await;
        }
        Ok(self.existing_response())
    }
    /// Waits until the request finished or failed; its error text.
    pub(crate) async fn settled(&self) -> Option<String> {
        loop {
            let notified = self.inner.changed.notified();
            {
                let s = self.inner.state.lock().unwrap();
                if s.finished {
                    return s.failure.clone();
                }
            }
            notified.await;
        }
    }
    /// `request.frame()`: the frame that made the request.
    pub async fn frame(&self) -> Result<Option<Frame>> {
        let Some(page) = self.inner.page.upgrade() else {
            return Ok(None);
        };
        Ok(Page { inner: page }
            .frames()
            .await?
            .into_iter()
            .find(|f| f.context == self.inner.context))
    }
    /// `request.serviceWorker()`: BiDi does not report service-worker
    /// requests separately; always `None`.
    pub fn service_worker(&self) -> Option<Worker> {
        None
    }
    /// `request.timing()` (complete once the response finished).
    pub fn timing(&self) -> Timing {
        self.inner
            .state
            .lock()
            .unwrap()
            .timing
            .unwrap_or_else(|| Timing::unknown(0.0))
    }
    /// `request.sizes()`: waits for the response to finish.
    pub async fn sizes(&self) -> Result<RequestSizes> {
        self.settled().await;
        let response = self.existing_response();
        let headers_size = if self.inner.headers_size > 0 {
            self.inner.headers_size
        } else {
            // The request line and the headers, as sent.
            let path = url::Url::parse(&self.inner.url)
                .map(|u| u[url::Position::BeforePath..].to_owned())
                .unwrap_or_default();
            let line = format!("{} {} HTTP/1.1\r\n", self.inner.method, path).len();
            let headers: usize = self
                .inner
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len() + 4)
                .sum();
            (line + headers + 2) as u64
        };
        Ok(RequestSizes {
            request_body_size: self
                .inner
                .post_data
                .as_ref()
                .map_or(self.inner.body_size, |b| b.len() as u64),
            request_headers_size: headers_size,
            response_body_size: response.as_ref().map_or(0, |r| r.data.body_size),
            response_headers_size: response.as_ref().map_or(0, |r| r.data.headers_size),
        })
    }
}

/// A response (`PageEvent::Response`, `request.response()`).
#[derive(Clone)]
pub struct Response {
    pub(crate) data: Arc<ResponseData>,
    request: Request,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("firefox::Response")
            .field("url", &self.data.url)
            .field("status", &self.data.status)
            .finish()
    }
}

impl Response {
    pub fn url(&self) -> &str {
        &self.data.url
    }
    pub fn status(&self) -> u16 {
        self.data.status
    }
    pub fn status_text(&self) -> &str {
        &self.data.status_text
    }
    /// `response.ok()`: status 200..=299 (or 0 for non-HTTP URLs).
    pub fn ok(&self) -> bool {
        self.data.status == 0 || (200..300).contains(&self.data.status)
    }
    pub fn headers(&self) -> &[(String, String)] {
        &self.data.headers
    }
    /// `response.headersArray()`.
    pub fn headers_array(&self) -> Vec<(String, String)> {
        self.data.headers.clone()
    }
    /// `response.allHeaders()`.
    pub async fn all_headers(&self) -> Result<HashMap<String, String>> {
        Ok(all_headers(&self.data.headers))
    }
    /// The first header with this name (case-insensitive).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        header_value(&self.data.headers, name)
    }
    /// `response.headerValues(name)`: every value of the header.
    pub fn header_values(&self, name: &str) -> Vec<String> {
        self.data
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
            .collect()
    }
    /// `response.request()`.
    pub fn request(&self) -> Request {
        self.request.clone()
    }
    /// `response.frame()`.
    pub async fn frame(&self) -> Result<Option<Frame>> {
        self.request.frame().await
    }
    /// `response.finished()`: waits for the body; the error text if the
    /// request failed.
    pub async fn finished(&self) -> Result<Option<String>> {
        Ok(self.request.settled().await)
    }
    /// `response.body()`: from the context's BiDi data collector.
    pub async fn body(&self) -> Result<Vec<u8>> {
        self.request.settled().await;
        let page = self
            .request
            .inner
            .page
            .upgrade()
            .map(|inner| Page { inner })
            .ok_or_else(|| Error::TargetClosed("page closed".into()))?;
        page.collected_data(&self.request.inner.id, "response")
            .await
            .ok_or_else(|| {
                Error::Evaluation(format!(
                    "Response body is unavailable for {}",
                    self.data.url
                ))
            })
    }
    /// `response.text()`.
    pub async fn text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.body().await?).into_owned())
    }
    /// `response.json()`.
    pub async fn json<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_slice(&self.body().await?)?)
    }
    /// `response.httpVersion()`.
    pub fn http_version(&self) -> String {
        http_version_of(&self.data.protocol)
    }
    /// `response.fromServiceWorker()`: BiDi does not say; always false.
    pub fn from_service_worker(&self) -> bool {
        false
    }
    /// `response.serverAddr()`: WebDriver BiDi does not report it.
    pub fn server_addr(&self) -> Option<ServerAddr> {
        None
    }
    /// `response.securityDetails()`: WebDriver BiDi does not report it.
    pub fn security_details(&self) -> Option<SecurityDetails> {
        None
    }
}
