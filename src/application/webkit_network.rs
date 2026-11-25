//! Requests and responses on the WebKit engine: Playwright's `Request` and
//! `Response`, kept up to date by WebKit `Network.*` events (response,
//! completion or failure, redirects, timing, sizes, server address), as in
//! Playwright's `WKInterceptableRequest`. Bodies come from
//! `Network.getResponseBody`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use base64::Engine;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::application::network::{SecurityDetails, ServerAddr};
use crate::application::webkit::{Page, PageInner};
use crate::application::webkit_frame::Frame;
use crate::application::webkit_worker::Worker;
use crate::domain::error::{Error, Result};
use crate::domain::firefox::http_version_of;
use crate::domain::network::{RequestSizes, Timing};

/// A network request (`PageEvent::Request` and friends, or a route's).
#[derive(Clone)]
pub struct Request {
    pub(crate) inner: Arc<RequestInner>,
}

pub(crate) struct RequestInner {
    /// WebKit's request id (shared by the hops of a redirect chain).
    pub(crate) id: String,
    /// The page target that reported it (for `Network.getResponseBody`).
    target: String,
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    resource_type: String,
    navigation: bool,
    frame: String,
    post_data: Option<Vec<u8>>,
    /// Monotonic send time (s), for `responseEnd`.
    timestamp: f64,
    /// Wall-clock send time (s), for `startTime`.
    wall_time: f64,
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
    metrics: Value,
}

/// A response's status line, headers and TLS details.
#[derive(Debug)]
pub(crate) struct ResponseData {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
    security: Value,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("webkit::Request")
            .field("url", &self.inner.url)
            .field("method", &self.inner.method)
            .finish()
    }
}

/// WebKit `Headers` (an object) as name / value pairs.
pub(crate) fn header_pairs(h: &Value) -> Vec<(String, String)> {
    h.as_object()
        .map(|h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned()))
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

/// WebKit's `postData` (base64).
fn post_data(v: &Value) -> Option<Vec<u8>> {
    let d = v.as_str()?;
    Some(
        base64::engine::general_purpose::STANDARD
            .decode(d)
            .unwrap_or_else(|_| d.as_bytes().to_vec()),
    )
}

/// Playwright's rounding of WebKit's millisecond timings.
fn roundish(v: f64) -> f64 {
    if v <= 0.0 {
        -1.0
    } else {
        (v * 1000.0).round() / 1000.0
    }
}

impl Request {
    /// From `Network.requestWillBeSent` on `target`.
    pub(crate) fn from_event(
        target: &str,
        p: &Value,
        page: Weak<PageInner>,
        redirected_from: Option<Request>,
    ) -> Request {
        let r = &p["request"];
        let resource_type = p["type"].as_str().unwrap_or("Other").to_ascii_lowercase();
        Request {
            inner: Arc::new(RequestInner {
                id: p["requestId"].as_str().unwrap_or_default().to_owned(),
                target: target.to_owned(),
                url: r["url"].as_str().unwrap_or_default().to_owned(),
                method: r["method"].as_str().unwrap_or("GET").to_owned(),
                headers: header_pairs(&r["headers"]),
                navigation: resource_type == "document",
                resource_type,
                frame: p["frameId"].as_str().unwrap_or_default().to_owned(),
                post_data: post_data(&r["postData"]),
                timestamp: p["timestamp"].as_f64().unwrap_or_default(),
                wall_time: p["walltime"].as_f64().unwrap_or_default(),
                redirected_from,
                page,
                state: Mutex::new(State::default()),
                changed: tokio::sync::Notify::new(),
            }),
        }
    }

    /// The request a route intercepted (`Network.requestIntercepted`).
    pub(crate) fn intercepted(target: &str, p: &Value, page: Weak<PageInner>) -> Request {
        let r = &p["request"];
        Request {
            inner: Arc::new(RequestInner {
                id: p["requestId"].as_str().unwrap_or_default().to_owned(),
                target: target.to_owned(),
                url: r["url"].as_str().unwrap_or_default().to_owned(),
                method: r["method"].as_str().unwrap_or("GET").to_owned(),
                headers: header_pairs(&r["headers"]),
                resource_type: "other".into(),
                navigation: false,
                frame: String::new(),
                post_data: post_data(&r["postData"]),
                timestamp: 0.0,
                wall_time: 0.0,
                redirected_from: None,
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

    /// A response arrived (`Network.responseReceived`, or the
    /// `redirectResponse` of the next hop); its `Response`.
    pub(crate) fn responded(&self, r: &Value) -> Response {
        let data = Arc::new(ResponseData {
            url: r["url"]
                .as_str()
                .filter(|u| !u.is_empty())
                .unwrap_or(&self.inner.url)
                .to_owned(),
            status: r["status"].as_u64().unwrap_or_default() as u16,
            status_text: r["statusText"].as_str().unwrap_or_default().to_owned(),
            headers: header_pairs(&r["headers"]),
            security: r["security"].clone(),
        });
        let t = &r["timing"];
        let f = |k: &str| roundish(t[k].as_f64().unwrap_or(-1.0));
        let timing = Timing {
            start_time: self.inner.wall_time * 1000.0,
            domain_lookup_start: f("domainLookupStart"),
            domain_lookup_end: f("domainLookupEnd"),
            connect_start: f("connectStart"),
            secure_connection_start: f("secureConnectionStart"),
            connect_end: f("connectEnd"),
            request_start: f("requestStart"),
            response_start: f("responseStart"),
            response_end: -1.0,
        };
        self.update(|s| {
            s.response = Some(data.clone());
            s.timing = Some(timing);
        });
        Response {
            data,
            request: self.clone(),
        }
    }

    /// The response finished at monotonic `timestamp` (s), with WebKit's
    /// `metrics` (`Network.loadingFinished`).
    pub(crate) fn completed(&self, timestamp: f64, metrics: &Value) {
        let start = self.inner.timestamp;
        self.update(|s| {
            let mut t = s
                .timing
                .unwrap_or_else(|| Timing::unknown(self.inner.wall_time));
            t.finish(start, timestamp);
            s.timing = Some(t);
            s.metrics = metrics.clone();
            s.finished = true;
        });
    }

    /// The request failed (`Network.loadingFailed`).
    pub(crate) fn failed(&self, error: &str) {
        self.update(|s| {
            s.failure = Some(error.to_owned());
            s.finished = true;
        });
    }

    /// The next hop of a redirect, at monotonic `timestamp`.
    pub(crate) fn redirected(&self, to: &Request, timestamp: f64) {
        self.completed(timestamp, &Value::Null);
        self.update(|s| s.redirected_to = Some(to.clone()));
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
        &self.inner.resource_type
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
    fn page(&self) -> Option<Page> {
        self.inner.page.upgrade().map(|inner| Page { inner })
    }
    /// `request.frame()`: the frame that made the request.
    pub async fn frame(&self) -> Result<Option<Frame>> {
        let Some(page) = self.page() else {
            return Ok(None);
        };
        Ok(page
            .frames()
            .await?
            .into_iter()
            .find(|f| f.id == self.inner.frame))
    }
    /// `request.serviceWorker()`: Playwright reports service-worker
    /// requests in Chromium only; always `None`.
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
            .unwrap_or_else(|| Timing::unknown(self.inner.wall_time))
    }
    /// `request.sizes()`: waits for the response to finish (WebKit's
    /// network metrics).
    pub async fn sizes(&self) -> Result<RequestSizes> {
        self.settled().await;
        let m = self.inner.state.lock().unwrap().metrics.clone();
        let n = |k: &str| m[k].as_f64().unwrap_or_default().max(0.0) as u64;
        Ok(RequestSizes {
            request_body_size: m["requestBodyBytesSent"]
                .as_f64()
                .map(|v| v as u64)
                .unwrap_or_else(|| self.inner.post_data.as_ref().map_or(0, |b| b.len() as u64)),
            request_headers_size: n("requestHeaderBytesSent"),
            response_body_size: n("responseBodyBytesReceived"),
            response_headers_size: n("responseHeaderBytesReceived"),
        })
    }
}

impl Request {
    /// The request (and response) in WebDriver BiDi's shape, for the HAR
    /// entry builder shared with Firefox.
    pub(crate) fn har_json(&self, response: Option<&Response>) -> (Value, Option<Value>) {
        let headers = |h: &[(String, String)]| -> Vec<Value> {
            h.iter()
                .map(|(k, v)| json!({"name": k, "value": {"type": "string", "value": v}}))
                .collect()
        };
        let t = self.timing();
        let start = t.start_time;
        let at = |rel: f64| if rel >= 0.0 { start + rel } else { 0.0 };
        let m = self.inner.state.lock().unwrap().metrics.clone();
        let num = |k: &str| m[k].as_f64().map_or(-1, |v| v as i64);
        let request = json!({
            "url": self.inner.url,
            "method": self.inner.method,
            "headers": headers(&self.inner.headers),
            "headersSize": num("requestHeaderBytesSent"),
            "bodySize": self.inner.post_data.as_ref().map_or(0, |b| b.len() as i64),
            "timings": {
                "requestTime": start,
                "dnsStart": at(t.domain_lookup_start), "dnsEnd": at(t.domain_lookup_end),
                "connectStart": at(t.connect_start), "connectEnd": at(t.connect_end),
                "tlsStart": at(t.secure_connection_start),
                "requestStart": at(t.request_start), "responseStart": at(t.response_start),
                "responseEnd": at(t.response_end),
            },
        });
        let response = response.map(|r| {
            json!({
                "status": r.status(),
                "statusText": r.status_text(),
                "headers": headers(r.headers()),
                "mimeType": r.header_value("content-type").unwrap_or_default(),
                "protocol": m["protocol"].as_str().unwrap_or("http/1.1"),
                "headersSize": num("responseHeaderBytesReceived"),
                "bodySize": num("responseBodyBytesReceived"),
            })
        });
        (request, response)
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
        f.debug_struct("webkit::Response")
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
    /// `response.body()` (`Network.getResponseBody`).
    pub async fn body(&self) -> Result<Vec<u8>> {
        self.request.settled().await;
        let page = self
            .request
            .page()
            .ok_or_else(|| Error::TargetClosed("page closed".into()))?;
        let r = page
            .inner
            .wk
            .call_target(
                &page.inner.proxy,
                &self.request.inner.target,
                "Network.getResponseBody",
                json!({"requestId": self.request.inner.id}),
            )
            .await
            .map_err(|_| {
                Error::Evaluation(format!(
                    "Response body is unavailable for {}",
                    self.data.url
                ))
            })?;
        let body = r["body"].as_str().unwrap_or_default();
        if r["base64Encoded"].as_bool() == Some(true) {
            base64::engine::general_purpose::STANDARD
                .decode(body)
                .map_err(|e| Error::Serde(format!("response body: {e}")))
        } else {
            Ok(body.as_bytes().to_vec())
        }
    }
    /// `response.text()`.
    pub async fn text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.body().await?).into_owned())
    }
    /// `response.json()`.
    pub async fn json<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_slice(&self.body().await?)?)
    }
    /// `response.httpVersion()` (from WebKit's metrics once finished).
    pub fn http_version(&self) -> String {
        let m = self.request.inner.state.lock().unwrap().metrics.clone();
        http_version_of(m["protocol"].as_str().unwrap_or_default())
    }
    /// `response.fromServiceWorker()`: Chromium only; false.
    pub fn from_service_worker(&self) -> bool {
        false
    }
    /// `response.serverAddr()`: WebKit's `remoteAddress` (once finished).
    pub fn server_addr(&self) -> Option<ServerAddr> {
        let m = self.request.inner.state.lock().unwrap().metrics.clone();
        let addr = m["remoteAddress"].as_str()?;
        let (host, port) = addr.rsplit_once(':')?;
        Some(ServerAddr {
            ip_address: host
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port: port.parse().ok()?,
        })
    }
    /// `response.securityDetails()`: the TLS certificate WebKit reports.
    pub fn security_details(&self) -> Option<SecurityDetails> {
        let s = &self.data.security;
        let cert = s.get("certificate")?;
        Some(SecurityDetails {
            protocol: s
                .pointer("/connection/protocol")
                .and_then(Value::as_str)
                .map(str::to_owned),
            subject_name: cert["subject"].as_str().map(str::to_owned),
            issuer: None,
            valid_from: cert["validFrom"].as_f64(),
            valid_to: cert["validUntil"].as_f64(),
        })
    }
}
