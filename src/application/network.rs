//! Network requests and responses (Playwright's `Request` / `Response`) and
//! the page's network events.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::watch;

use crate::application::events::{EventWaiter, PageEvent};
use crate::application::frame::Frame;
use crate::application::page::{Page, PageInner};
use crate::domain::error::{Error, Result};
use crate::domain::network::{RawHeaders, Timing, resource_type};
use crate::domain::url_matcher::UrlMatcher;
use crate::ports::RawEvent;

/// Where a response came from (`response.serverAddr()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddr {
    pub ip_address: String,
    pub port: u16,
}

/// TLS details of a response (`response.securityDetails()`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SecurityDetails {
    pub protocol: Option<String>,
    pub subject_name: Option<String>,
    pub issuer: Option<String>,
    pub valid_from: Option<f64>,
    pub valid_to: Option<f64>,
}

/// A network request made by a page. Cheap to clone.
#[derive(Clone)]
pub struct Request {
    inner: Arc<RequestInner>,
}

struct RequestInner {
    page: Weak<PageInner>,
    request_id: String,
    url: String,
    method: String,
    headers: RawHeaders,
    post_data: Option<Vec<u8>>,
    resource_type: String,
    is_navigation: bool,
    frame_id: Option<String>,
    redirected_from: Option<Request>,
    timestamp: f64,
    state: Mutex<RequestState>,
    changed: watch::Sender<u64>,
}

struct RequestState {
    redirected_to: Option<Weak<RequestInner>>,
    response: Option<Arc<ResponseData>>,
    raw_headers: Option<RawHeaders>,
    failure: Option<String>,
    done: bool,
    timing: Timing,
    /// Bytes received (headers and body), from `loadingFinished`.
    encoded_data_length: Option<f64>,
}

struct ResponseData {
    url: String,
    status: u16,
    status_text: String,
    headers: RawHeaders,
    has_extra_info: bool,
    is_redirect: bool,
    server_addr: Option<ServerAddr>,
    security_details: Option<SecurityDetails>,
    from_service_worker: bool,
    raw_headers: Mutex<Option<RawHeaders>>,
    /// `protocol` of the response (`http/1.1`, `h2`, ...).
    protocol: Option<String>,
    /// Length of the raw response head, when the browser reported it.
    headers_text_len: Mutex<Option<usize>>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.inner.method)
            .field("url", &self.inner.url)
            .finish()
    }
}

impl PartialEq for Request {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Request {
    pub fn url(&self) -> &str {
        &self.inner.url
    }
    pub fn method(&self) -> &str {
        &self.inner.method
    }
    /// `document`, `stylesheet`, `image`, `fetch`, `xhr`, `script`, ...
    pub fn resource_type(&self) -> &str {
        &self.inner.resource_type
    }
    pub fn is_navigation_request(&self) -> bool {
        self.inner.is_navigation
    }

    /// The headers known when the request was issued (lower-case names).
    pub fn headers(&self) -> HashMap<String, String> {
        self.inner.headers.to_map()
    }

    /// The request headers as sent on the wire, once known.
    pub async fn all_headers(&self) -> Result<HashMap<String, String>> {
        Ok(self.raw_headers().await.to_map())
    }

    /// `request.headersArray()`.
    pub async fn headers_array(&self) -> Result<Vec<crate::domain::network::HeaderEntry>> {
        Ok(self.raw_headers().await.entries().to_vec())
    }

    /// `request.headerValue(name)`.
    pub async fn header_value(&self, name: &str) -> Result<Option<String>> {
        Ok(self.raw_headers().await.get(name))
    }

    async fn raw_headers(&self) -> RawHeaders {
        self.wait(|st| {
            if let Some(raw) = &st.raw_headers {
                return Some(raw.clone());
            }
            let no_extra = st.response.as_ref().is_some_and(|r| !r.has_extra_info);
            (no_extra || st.done).then(|| self.inner.headers.clone())
        })
        .await
        .unwrap_or_else(|| self.inner.headers.clone())
    }

    /// The request body as text.
    pub fn post_data(&self) -> Option<String> {
        self.inner
            .post_data
            .as_ref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
    }

    /// The request body as bytes.
    pub fn post_data_buffer(&self) -> Option<Vec<u8>> {
        self.inner.post_data.clone()
    }

    /// The request body parsed as JSON (or as a form when url-encoded).
    pub fn post_data_json<T: DeserializeOwned>(&self) -> Result<Option<T>> {
        let Some(text) = self.post_data() else {
            return Ok(None);
        };
        let is_form = self
            .inner
            .headers
            .get("content-type")
            .is_some_and(|c| c.contains("application/x-www-form-urlencoded"));
        let value = if is_form {
            let map: serde_json::Map<String, Value> = text
                .split('&')
                .filter(|s| !s.is_empty())
                .map(|pair| {
                    let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                    (form_decode(k), Value::String(form_decode(v)))
                })
                .collect();
            Value::Object(map)
        } else {
            serde_json::from_str(&text).map_err(|e| {
                Error::InvalidArgument(format!("POST data is not a valid JSON object: {e}"))
            })?
        };
        Ok(Some(serde_json::from_value(value)?))
    }

    /// The frame that issued the request.
    pub fn frame(&self) -> Option<Frame> {
        let page = self.page()?;
        self.inner
            .frame_id
            .as_deref()
            .map(|id| page.frame_by_id(id))
    }

    /// The request this one was redirected from.
    pub fn redirected_from(&self) -> Option<Request> {
        self.inner.redirected_from.clone()
    }

    /// The request the server redirected this one to.
    pub fn redirected_to(&self) -> Option<Request> {
        let st = self.inner.state.lock().unwrap();
        st.redirected_to
            .as_ref()
            .and_then(Weak::upgrade)
            .map(|inner| Request { inner })
    }

    /// The error text (`net::ERR_...`) if the request failed.
    pub fn failure(&self) -> Option<String> {
        self.inner.state.lock().unwrap().failure.clone()
    }

    /// The response, once received; `None` if the request failed.
    pub async fn response(&self) -> Result<Option<Response>> {
        let data = self
            .wait(|st| match &st.response {
                Some(r) => Some(Some(r.clone())),
                None if st.done => Some(None),
                None => None,
            })
            .await
            .flatten();
        Ok(data.map(|d| Response::new(self.clone(), d)))
    }

    /// `request.existingResponse()`: the response if it has arrived already.
    pub fn existing_response(&self) -> Option<Response> {
        let data = self.inner.state.lock().unwrap().response.clone()?;
        Some(Response::new(self.clone(), data))
    }

    /// `request.sizes()`: header and body sizes, once the response finished.
    pub async fn sizes(&self) -> Result<crate::domain::network::RequestSizes> {
        let Some(response) = self.response().await? else {
            return Err(Error::InvalidArgument(
                "Unable to fetch sizes for failed request".into(),
            ));
        };
        response.finished().await?;
        let raw_request = self.raw_headers().await;
        let request_headers_size = crate::domain::network::request_headers_size(
            raw_request.entries(),
            self.url(),
            self.method(),
        );
        let raw_response = response.raw_headers().await;
        let text_len = *response.data.headers_text_len.lock().unwrap();
        let response_headers_size = match text_len {
            Some(n) => n as u64,
            None => crate::domain::network::response_headers_size(
                raw_response.entries(),
                &response.status_text,
            ),
        };
        let encoded = self.inner.state.lock().unwrap().encoded_data_length;
        let response_body_size = match encoded {
            Some(total) if !response.data.is_redirect => {
                (total as u64).saturating_sub(response_headers_size)
            }
            _ => raw_response
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        };
        Ok(crate::domain::network::RequestSizes {
            request_body_size: self.inner.post_data.as_ref().map_or(0, |b| b.len() as u64),
            request_headers_size,
            response_body_size,
            response_headers_size,
        })
    }

    /// `request.timing()`.
    pub fn timing(&self) -> Timing {
        self.inner.state.lock().unwrap().timing
    }

    fn page(&self) -> Option<Page> {
        self.inner.page.upgrade().map(|inner| Page { inner })
    }

    fn update(&self, f: impl FnOnce(&mut RequestState)) {
        f(&mut self.inner.state.lock().unwrap());
        self.inner.changed.send_modify(|v| *v += 1);
    }

    /// Waits until `f` yields a value; `None` if the page went away first.
    async fn wait<T>(&self, f: impl Fn(&RequestState) -> Option<T>) -> Option<T> {
        let mut rx = self.inner.changed.subscribe();
        loop {
            if let Some(v) = f(&self.inner.state.lock().unwrap()) {
                return Some(v);
            }
            if self.page().is_none_or(|p| p.is_closed()) || rx.changed().await.is_err() {
                return f(&self.inner.state.lock().unwrap());
            }
        }
    }
}

fn form_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(b) => {
                    out.push(b);
                    i += 2;
                }
                Err(_) => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A response to a [`Request`]. The public fields keep the 0.10 navigation
/// API; the methods follow Playwright's `Response`.
#[derive(Clone)]
pub struct Response {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    /// Response headers with lower-case names (no `set-cookie`, like
    /// Playwright's `response.headers()`); see [`Response::all_headers`].
    pub headers: HashMap<String, String>,
    request: Request,
    data: Arc<ResponseData>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("url", &self.url)
            .field("status", &self.status)
            .field("status_text", &self.status_text)
            .field("headers", &self.headers)
            .finish()
    }
}

impl PartialEq for Response {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
            && self.status == other.status
            && self.status_text == other.status_text
            && self.headers == other.headers
    }
}

impl Response {
    fn new(request: Request, data: Arc<ResponseData>) -> Self {
        Response {
            url: data.url.clone(),
            status: data.status,
            status_text: data.status_text.clone(),
            headers: data.headers.to_map(),
            request,
            data,
        }
    }

    /// Status in 200..=299.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The request this responds to.
    pub fn request(&self) -> Request {
        self.request.clone()
    }

    /// The frame that issued the request.
    pub fn frame(&self) -> Option<Frame> {
        self.request.frame()
    }

    /// `response.httpVersion()`: `HTTP/1.1`, `HTTP/2.0`, ...
    pub async fn http_version(&self) -> Result<String> {
        Ok(crate::domain::network::http_version(
            self.data.protocol.as_deref(),
        ))
    }

    pub fn from_service_worker(&self) -> bool {
        self.data.from_service_worker
    }

    pub fn server_addr(&self) -> Option<ServerAddr> {
        self.data.server_addr.clone()
    }

    pub fn security_details(&self) -> Option<SecurityDetails> {
        self.data.security_details.clone()
    }

    async fn raw_headers(&self) -> RawHeaders {
        if self.data.has_extra_info {
            let mut rx = self.request.inner.changed.subscribe();
            let page = self.request.page();
            let deadline = page.as_ref().map(|p| p.default_timeout());
            let wait = async {
                loop {
                    if let Some(raw) = self.data.raw_headers.lock().unwrap().clone() {
                        return Some(raw);
                    }
                    if rx.changed().await.is_err() {
                        return None;
                    }
                }
            };
            if let (Some(page), Some(timeout)) = (page, deadline)
                && let Ok(Some(raw)) = crate::application::wait::with_timeout(
                    &*page.inner.shared.services.clock,
                    timeout,
                    "waiting for raw response headers",
                    async { Ok(wait.await) },
                )
                .await
            {
                return raw;
            }
        }
        self.data
            .raw_headers
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| self.data.headers.clone())
    }

    /// All response headers as received, including `set-cookie`.
    pub async fn all_headers(&self) -> Result<HashMap<String, String>> {
        Ok(self.raw_headers().await.to_map())
    }

    /// `response.headersArray()`.
    pub async fn headers_array(&self) -> Result<Vec<crate::domain::network::HeaderEntry>> {
        Ok(self.raw_headers().await.entries().to_vec())
    }

    /// `response.headerValue(name)`: values joined with `, ` (`\n` for set-cookie).
    pub async fn header_value(&self, name: &str) -> Result<Option<String>> {
        Ok(self.raw_headers().await.get(name))
    }

    /// `response.headerValues(name)`.
    pub async fn header_values(&self, name: &str) -> Result<Vec<String>> {
        Ok(self.raw_headers().await.get_all(name))
    }

    /// Waits for the response body to finish; the failure text if it failed.
    pub async fn finished(&self) -> Result<Option<String>> {
        self.request.wait(|st| st.done.then_some(())).await;
        Ok(self.request.failure())
    }

    /// The response body.
    pub async fn body(&self) -> Result<Vec<u8>> {
        if self.data.is_redirect {
            return Err(Error::InvalidArgument(
                "Response body is unavailable for redirect responses".into(),
            ));
        }
        self.request.wait(|st| st.done.then_some(())).await;
        let page = self
            .request
            .page()
            .ok_or_else(|| Error::TargetClosed("page has been closed".into()))?;
        let r = page
            .send(
                "Network.getResponseBody",
                json!({"requestId": self.request.inner.request_id}),
            )
            .await?;
        let body = r.get("body").and_then(Value::as_str).unwrap_or_default();
        if r.get("base64Encoded").and_then(Value::as_bool) == Some(true) {
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .map_err(|e| Error::Evaluation(format!("invalid response body: {e}")))
        } else {
            Ok(body.as_bytes().to_vec())
        }
    }

    /// The body as UTF-8 text.
    pub async fn text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.body().await?).into_owned())
    }

    /// The body parsed as JSON.
    pub async fn json<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_slice(&self.body().await?)?)
    }
}

/// In-flight requests of a page, keyed by protocol request id.
#[derive(Default)]
pub(crate) struct NetworkState {
    requests: HashMap<String, Request>,
    /// Raw response headers that arrived before their response.
    early_response_extra: HashMap<String, VecDeque<(RawHeaders, Option<usize>)>>,
    /// Responses still waiting for their raw headers.
    awaiting_response_extra: HashMap<String, VecDeque<(Request, Arc<ResponseData>)>>,
    /// Raw request headers that arrived before their request.
    early_request_extra: HashMap<String, VecDeque<RawHeaders>>,
    /// `Fetch.requestPaused` interception ids that arrived before their request.
    paused: HashMap<String, String>,
    /// The most recent requests (`page.requests()`).
    recent: VecDeque<Request>,
}

/// How many requests `page.requests()` remembers.
const RECENT_REQUESTS: usize = 100;

impl NetworkState {
    pub(crate) fn request(&self, id: &str) -> Option<Request> {
        self.requests.get(id).cloned()
    }

    pub(crate) fn stash_paused(&mut self, network_id: &str, interception_id: String) {
        self.paused.insert(network_id.to_owned(), interception_id);
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn response_data(r: &Value, is_redirect: bool, has_extra_info: bool) -> ResponseData {
    let server_addr = match (
        r.get("remoteIPAddress").and_then(Value::as_str),
        r.get("remotePort").and_then(Value::as_u64),
    ) {
        (Some(ip), Some(port)) => Some(ServerAddr {
            ip_address: ip.to_owned(),
            port: port as u16,
        }),
        _ => None,
    };
    let security_details = r.get("securityDetails").map(|d| SecurityDetails {
        protocol: d.get("protocol").and_then(Value::as_str).map(str::to_owned),
        subject_name: d
            .get("subjectName")
            .and_then(Value::as_str)
            .map(str::to_owned),
        issuer: d.get("issuer").and_then(Value::as_str).map(str::to_owned),
        valid_from: d.get("validFrom").and_then(Value::as_f64),
        valid_to: d.get("validTo").and_then(Value::as_f64),
    });
    ResponseData {
        url: s(r, "url"),
        status: r.get("status").and_then(Value::as_u64).unwrap_or_default() as u16,
        status_text: s(r, "statusText"),
        headers: RawHeaders::from_protocol(&r["headers"], false),
        has_extra_info,
        is_redirect,
        server_addr,
        security_details,
        from_service_worker: r.get("fromServiceWorker").and_then(Value::as_bool) == Some(true),
        protocol: r.get("protocol").and_then(Value::as_str).map(str::to_owned),
        headers_text_len: Mutex::new(None),
        raw_headers: Mutex::new(None),
    }
}

impl Page {
    /// Updates the network model and emits request/response events.
    pub(crate) fn handle_network_event(&self, e: &RawEvent) {
        let p = &e.params;
        let mut emit = Vec::new();
        let mut route = None;
        {
            let mut net = self.inner.network.lock().unwrap();
            match e.method.as_str() {
                "Network.requestWillBeSent" => {
                    let url = p
                        .pointer("/request/url")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if url.starts_with("data:") {
                        return;
                    }
                    let id = s(p, "requestId");
                    let timestamp = p.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
                    let mut redirected_from = None;
                    if let Some(redirect) = p.get("redirectResponse")
                        && let Some(prev) = net.requests.remove(&id)
                    {
                        let has_extra =
                            p.get("redirectHasExtraInfo").and_then(Value::as_bool) == Some(true);
                        let data = self.attach_response(&mut net, &prev, redirect, true, has_extra);
                        prev.update(|st| {
                            st.timing.finish(prev.inner.timestamp, timestamp);
                            st.done = true;
                        });
                        emit.push(PageEvent::Response(Response::new(prev.clone(), data)));
                        emit.push(PageEvent::RequestFinished(prev.clone()));
                        redirected_from = Some(prev);
                    }
                    let req = &p["request"];
                    let post_data = req
                        .get("postDataEntries")
                        .and_then(Value::as_array)
                        .map(|entries| {
                            entries
                                .iter()
                                .filter_map(|e| e.get("bytes").and_then(Value::as_str))
                                .filter_map(|b| {
                                    base64::engine::general_purpose::STANDARD.decode(b).ok()
                                })
                                .flatten()
                                .collect::<Vec<u8>>()
                        })
                        .or_else(|| {
                            req.get("postData")
                                .and_then(Value::as_str)
                                .map(|d| d.as_bytes().to_vec())
                        });
                    let wall_time = p.get("wallTime").and_then(Value::as_f64).unwrap_or(0.0);
                    let raw_headers = net
                        .early_request_extra
                        .get_mut(&id)
                        .and_then(VecDeque::pop_front);
                    let (changed, _) = watch::channel(0);
                    let request = Request {
                        inner: Arc::new(RequestInner {
                            page: Arc::downgrade(&self.inner),
                            request_id: id.clone(),
                            url: format!("{url}{}", s(req, "urlFragment")),
                            method: s(req, "method"),
                            headers: RawHeaders::from_protocol(&req["headers"], false),
                            post_data,
                            resource_type: resource_type(p.get("type").and_then(Value::as_str)),
                            is_navigation: s(p, "type") == "Document" && id == s(p, "loaderId"),
                            frame_id: p.get("frameId").and_then(Value::as_str).map(str::to_owned),
                            redirected_from: redirected_from.clone(),
                            timestamp,
                            state: Mutex::new(RequestState {
                                redirected_to: None,
                                response: None,
                                raw_headers,
                                failure: None,
                                encoded_data_length: None,
                                done: false,
                                timing: Timing::unknown(wall_time),
                            }),
                            changed,
                        }),
                    };
                    if let Some(prev) = &redirected_from {
                        prev.update(|st| st.redirected_to = Some(Arc::downgrade(&request.inner)));
                    }
                    net.requests.insert(id.clone(), request.clone());
                    if net.recent.len() == RECENT_REQUESTS {
                        net.recent.pop_front();
                    }
                    net.recent.push_back(request.clone());
                    if let Some(interception) = net.paused.remove(&id) {
                        route = Some((request.clone(), interception));
                    }
                    emit.push(PageEvent::Request(request));
                }
                "Network.requestWillBeSentExtraInfo" => {
                    let id = s(p, "requestId");
                    let raw = RawHeaders::from_protocol(&p["headers"], true);
                    match net.requests.get(&id) {
                        Some(r) if r.inner.state.lock().unwrap().raw_headers.is_none() => {
                            r.update(|st| st.raw_headers = Some(raw));
                        }
                        _ => net
                            .early_request_extra
                            .entry(id)
                            .or_default()
                            .push_back(raw),
                    }
                }
                "Network.responseReceivedExtraInfo" => {
                    let id = s(p, "requestId");
                    let raw = RawHeaders::from_protocol(&p["headers"], true);
                    let text_len = p.get("headersText").and_then(Value::as_str).map(str::len);
                    let waiting = net
                        .awaiting_response_extra
                        .get_mut(&id)
                        .and_then(VecDeque::pop_front);
                    match waiting {
                        Some((request, data)) => {
                            *data.headers_text_len.lock().unwrap() = text_len;
                            *data.raw_headers.lock().unwrap() = Some(raw);
                            request.update(|_| {});
                        }
                        None => net
                            .early_response_extra
                            .entry(id)
                            .or_default()
                            .push_back((raw, text_len)),
                    }
                    if net
                        .awaiting_response_extra
                        .get(&s(p, "requestId"))
                        .is_some_and(VecDeque::is_empty)
                    {
                        net.awaiting_response_extra.remove(&s(p, "requestId"));
                    }
                }
                "Network.responseReceived" => {
                    let id = s(p, "requestId");
                    let Some(request) = net.requests.get(&id).cloned() else {
                        return;
                    };
                    let has_extra = p.get("hasExtraInfo").and_then(Value::as_bool) == Some(true);
                    let data =
                        self.attach_response(&mut net, &request, &p["response"], false, has_extra);
                    let response = Response::new(request.clone(), data);
                    if request.inner.is_navigation {
                        self.inner
                            .state
                            .lock()
                            .unwrap()
                            .responses
                            .insert(id, response.clone());
                    }
                    emit.push(PageEvent::Response(response));
                }
                "Network.loadingFinished" => {
                    let id = s(p, "requestId");
                    net.early_request_extra.remove(&id);
                    let Some(request) = net.requests.remove(&id) else {
                        return;
                    };
                    let at = p.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
                    let encoded = p.get("encodedDataLength").and_then(Value::as_f64);
                    request.update(|st| {
                        if st.response.is_some() {
                            st.timing.finish(request.inner.timestamp, at);
                        }
                        st.encoded_data_length = encoded;
                        st.done = true;
                    });
                    emit.push(PageEvent::RequestFinished(request));
                }
                "Network.loadingFailed" => {
                    let id = s(p, "requestId");
                    net.early_request_extra.remove(&id);
                    let Some(request) = net.requests.remove(&id) else {
                        return;
                    };
                    let at = p.get("timestamp").and_then(Value::as_f64).unwrap_or(0.0);
                    let text = p
                        .get("errorText")
                        .and_then(Value::as_str)
                        .filter(|t| !t.is_empty())
                        .or_else(|| p.get("blockedReason").and_then(Value::as_str))
                        .unwrap_or_default()
                        .to_owned();
                    request.update(|st| {
                        if st.response.is_some() {
                            st.timing.finish(request.inner.timestamp, at);
                        }
                        st.failure = Some(text);
                        st.done = true;
                    });
                    emit.push(PageEvent::RequestFailed(request));
                }
                _ => return,
            }
        }
        for event in emit {
            self.emit(event);
        }
        if let Some((request, interception)) = route {
            self.dispatch_route(request, interception);
        }
    }

    fn attach_response(
        &self,
        net: &mut NetworkState,
        request: &Request,
        payload: &Value,
        is_redirect: bool,
        has_extra_info: bool,
    ) -> Arc<ResponseData> {
        let data = Arc::new(response_data(payload, is_redirect, has_extra_info));
        let id = &request.inner.request_id;
        if has_extra_info {
            match net
                .early_response_extra
                .get_mut(id)
                .and_then(VecDeque::pop_front)
            {
                Some((raw, text_len)) => {
                    *data.headers_text_len.lock().unwrap() = text_len;
                    *data.raw_headers.lock().unwrap() = Some(raw);
                }
                None => net
                    .awaiting_response_extra
                    .entry(id.clone())
                    .or_default()
                    .push_back((request.clone(), data.clone())),
            }
        }
        let wall_time = request.inner.state.lock().unwrap().timing.start_time / 1000.0;
        let timing =
            Timing::from_protocol(payload.get("timing"), wall_time, request.inner.timestamp);
        request.update(|st| {
            st.response = Some(data.clone());
            st.timing = timing;
        });
        data
    }

    /// Settles in-flight requests when the page goes away.
    pub(crate) fn close_network(&self) {
        let requests: Vec<Request> = {
            let mut net = self.inner.network.lock().unwrap();
            net.awaiting_response_extra.clear();
            net.paused.clear();
            net.requests.drain().map(|(_, r)| r).collect()
        };
        for r in requests {
            r.update(|st| st.done = true);
        }
    }

    /// `page.requests()`: the most recent requests (up to 100).
    pub fn requests(&self) -> Vec<Request> {
        self.inner
            .network
            .lock()
            .unwrap()
            .recent
            .iter()
            .cloned()
            .collect()
    }

    /// `page.waitForRequest(url)`.
    pub fn wait_for_request(&self, url: impl Into<UrlMatcher>) -> EventWaiter<'_, Request> {
        let matcher = url.into();
        self.wait_for_request_matching(move |r| matcher.matches(r.url()))
    }

    /// `page.waitForRequest(predicate)`.
    pub fn wait_for_request_matching<F>(&self, predicate: F) -> EventWaiter<'_, Request>
    where
        F: Fn(&Request) -> bool + Send + Sync + 'static,
    {
        self.wait_for_event(move |e| match e {
            PageEvent::Request(r) if predicate(r) => Some(r.clone()),
            _ => None,
        })
    }

    /// `page.waitForResponse(url)`.
    pub fn wait_for_response(&self, url: impl Into<UrlMatcher>) -> EventWaiter<'_, Response> {
        let matcher = url.into();
        self.wait_for_response_matching(move |r| matcher.matches(&r.url))
    }

    /// `page.waitForResponse(predicate)`.
    pub fn wait_for_response_matching<F>(&self, predicate: F) -> EventWaiter<'_, Response>
    where
        F: Fn(&Response) -> bool + Send + Sync + 'static,
    {
        self.wait_for_event(move |e| match e {
            PageEvent::Response(r) if predicate(r) => Some(r.clone()),
            _ => None,
        })
    }
}
