//! `APIRequestContext` / `APIResponse`: HTTP requests outside the page, with
//! the browser context's cookies and options (Playwright's `fetch.ts`).

use std::collections::{BTreeMap, HashMap};
use std::future::IntoFuture;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::application::context::{BrowserContext, ContextInner};
use crate::application::page::Page;
use crate::application::services::Services;
use crate::domain::emulation::HttpCredentials;
use crate::domain::error::{Error, Result};
use crate::domain::http::{
    HttpRequest, HttpResponse, MultipartValue, append_params, form_urlencode, multipart_body,
    multipart_boundary, parse_set_cookie, redirect_request, status_error,
};
use crate::domain::network::HeaderEntry;
use crate::domain::storage::{Cookie, NewCookie, StorageState, filter_cookies};
use crate::domain::url_matcher::resolve_url;

const CLOSED: &str = "Target page, context or browser has been closed";

/// Options of a standalone `request.newContext()`.
#[derive(Debug, Clone, Default)]
pub struct ApiRequestOptions {
    pub base_url: Option<String>,
    pub extra_http_headers: BTreeMap<String, String>,
    pub http_credentials: Option<HttpCredentials>,
    pub user_agent: Option<String>,
    pub ignore_https_errors: bool,
    /// Per request (default 30 s; zero for none).
    pub timeout: Option<Duration>,
    pub fail_on_status_code: bool,
    /// Cookies to start with.
    pub storage_state: Option<StorageState>,
}

enum Jar {
    /// The browser context's cookies.
    Context(Weak<ContextInner>),
    /// Its own cookies (standalone contexts).
    Own(Mutex<Vec<Cookie>>),
}

struct Inner {
    services: Services,
    options: ApiRequestOptions,
    jar: Jar,
    /// The browser's default user agent, for context requests.
    browser_ua: Mutex<Option<String>>,
    disposed: AtomicBool,
}

/// `APIRequestContext`. Cheap to clone.
#[derive(Clone)]
pub struct ApiRequestContext {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ApiRequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiRequestContext")
            .field("base_url", &self.inner.options.base_url)
            .finish()
    }
}

impl BrowserContext {
    /// `context.request`: requests with this context's cookies, headers,
    /// credentials and base URL.
    pub fn request(&self) -> ApiRequestContext {
        let o = &self.inner.options;
        ApiRequestContext {
            inner: Arc::new(Inner {
                services: self.inner.shared.services.clone(),
                options: ApiRequestOptions {
                    base_url: o.base_url.clone(),
                    extra_http_headers: o.extra_http_headers.clone(),
                    http_credentials: o.http_credentials.clone(),
                    user_agent: o.user_agent.clone(),
                    ignore_https_errors: o.ignore_https_errors,
                    timeout: None,
                    fail_on_status_code: false,
                    storage_state: None,
                },
                jar: Jar::Context(Arc::downgrade(&self.inner)),
                browser_ua: Mutex::new(None),
                disposed: AtomicBool::new(false),
            }),
        }
    }
}

impl Page {
    /// `page.request`: the page's context's request context.
    pub fn request(&self) -> ApiRequestContext {
        self.context()
            .expect("a page belongs to a context while it is open")
            .request()
    }
}

impl ApiRequestContext {
    /// A standalone context (Playwright's `request.newContext()`).
    pub fn new(services: Services, options: ApiRequestOptions) -> Self {
        let cookies = options
            .storage_state
            .as_ref()
            .map(|s| s.cookies.clone())
            .unwrap_or_default();
        ApiRequestContext {
            inner: Arc::new(Inner {
                services,
                options,
                jar: Jar::Own(Mutex::new(cookies)),
                browser_ua: Mutex::new(None),
                disposed: AtomicBool::new(false),
            }),
        }
    }

    fn context(&self) -> Option<BrowserContext> {
        match &self.inner.jar {
            Jar::Context(w) => w.upgrade().map(|inner| BrowserContext { inner }),
            Jar::Own(_) => None,
        }
    }

    pub fn fetch(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "GET", url)
    }
    pub fn get(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "GET", url)
    }
    pub fn head(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "HEAD", url)
    }
    pub fn post(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "POST", url)
    }
    pub fn put(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "PUT", url)
    }
    pub fn patch(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "PATCH", url)
    }
    pub fn delete(&self, url: &str) -> ApiRequest<'_> {
        ApiRequest::new(self, "DELETE", url)
    }

    /// `request.storageState()`: its cookies (and, for a context's, the
    /// context's storage).
    pub async fn storage_state(&self) -> Result<StorageState> {
        match &self.inner.jar {
            Jar::Own(c) => Ok(StorageState {
                cookies: c.lock().unwrap().clone(),
                origins: Vec::new(),
            }),
            Jar::Context(_) => match self.context() {
                Some(ctx) => ctx.storage_state().await,
                None => Err(Error::Request(CLOSED.into())),
            },
        }
    }

    /// `request.dispose()`: later requests fail.
    pub async fn dispose(&self) {
        self.inner.disposed.store(true, Ordering::SeqCst);
    }

    async fn user_agent(&self) -> String {
        if let Some(ua) = &self.inner.options.user_agent {
            return ua.clone();
        }
        let Some(ctx) = self.context() else {
            return format!("chromiumoxide/{}", env!("CARGO_PKG_VERSION"));
        };
        if let Some(ua) = self.inner.browser_ua.lock().unwrap().clone() {
            return ua;
        }
        let ua = ctx
            .inner
            .shared
            .cdp
            .call(None, "Browser.getVersion", serde_json::json!({}))
            .await
            .ok()
            .and_then(|v| {
                v.get("userAgent")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        *self.inner.browser_ua.lock().unwrap() = Some(ua.clone());
        ua
    }

    async fn cookies_for(&self, url: &str) -> Result<Vec<Cookie>> {
        let now = self.inner.services.clock.wall_ms() / 1000.0;
        let alive = |c: &Cookie| c.expires < 0.0 || c.expires > now;
        match &self.inner.jar {
            Jar::Own(c) => {
                let all = c.lock().unwrap().clone();
                Ok(filter_cookies(all, &[url])
                    .into_iter()
                    .filter(alive)
                    .collect())
            }
            Jar::Context(_) => match self.context() {
                Some(ctx) => ctx.cookies_for(&[url]).await,
                None => Err(Error::Request(CLOSED.into())),
            },
        }
    }

    async fn store_cookies(&self, cookies: Vec<NewCookie>) -> Result<()> {
        if cookies.is_empty() {
            return Ok(());
        }
        match &self.inner.jar {
            Jar::Own(jar) => {
                let mut jar = jar.lock().unwrap();
                for c in cookies {
                    let domain = c.domain.clone().unwrap_or_default();
                    let path = c.path.clone().unwrap_or_else(|| "/".into());
                    jar.retain(|o| !(o.name == c.name && o.domain == domain && o.path == path));
                    jar.push(Cookie {
                        name: c.name,
                        value: c.value,
                        domain,
                        path,
                        expires: c.expires.unwrap_or(-1.0),
                        http_only: c.http_only.unwrap_or(false),
                        secure: c.secure.unwrap_or(false),
                        same_site: c.same_site.unwrap_or_default(),
                        partition_key: None,
                    });
                }
                Ok(())
            }
            Jar::Context(_) => match self.context() {
                Some(ctx) => {
                    for c in cookies {
                        // One by one, like Playwright: a bad cookie does not drop the rest.
                        let _ = ctx.add_cookies(&[c]).await;
                    }
                    Ok(())
                }
                None => Err(Error::Request(CLOSED.into())),
            },
        }
    }

    fn credentials_for(&self, url: &str) -> Option<&HttpCredentials> {
        let c = self.inner.options.http_credentials.as_ref()?;
        match &c.origin {
            None => Some(c),
            Some(o) => {
                let origin = url::Url::parse(url).ok()?.origin().ascii_serialization();
                origin.eq_ignore_ascii_case(o).then_some(c)
            }
        }
    }

    async fn run(&self, r: ApiRequest<'_>) -> Result<ApiResponse> {
        if self.inner.disposed.load(Ordering::SeqCst)
            || matches!(self.inner.jar, Jar::Context(_)) && self.context().is_none()
        {
            return Err(Error::Request(CLOSED.into()));
        }
        let o = &self.inner.options;
        let url = append_params(&resolve_url(o.base_url.as_deref(), &r.url), &r.params);
        let mut headers: Vec<(String, String)> = vec![
            ("user-agent".into(), self.user_agent().await),
            ("accept".into(), "*/*".into()),
        ];
        let set = |headers: &mut Vec<(String, String)>, k: &str, v: &str| {
            headers.retain(|(n, _)| !n.eq_ignore_ascii_case(k));
            headers.push((k.to_owned(), v.to_owned()));
        };
        for (k, v) in &o.extra_http_headers {
            set(&mut headers, k, v);
        }
        for (k, v) in &r.headers {
            set(&mut headers, k, v);
        }
        let has = |headers: &[(String, String)], k: &str| {
            headers.iter().any(|(n, _)| n.eq_ignore_ascii_case(k))
        };
        let (body, content_type) = match r.body {
            Body::None => (Vec::new(), None),
            Body::Json(v) => (serde_json::to_vec(&v)?, Some("application/json".to_owned())),
            Body::Bytes(b) => (b, Some("application/octet-stream".to_owned())),
            Body::Form(pairs) => (
                form_urlencode(&pairs).into_bytes(),
                Some("application/x-www-form-urlencoded".to_owned()),
            ),
            Body::Multipart(fields) => {
                let mut random = [0u8; 16];
                let seed = self.inner.services.clock.wall_ms().to_bits()
                    ^ (self.inner.services.clock.now_ms().rotate_left(17));
                for (i, b) in random.iter_mut().enumerate() {
                    *b = (seed.rotate_left(i as u32 * 7) as u8)
                        .wrapping_mul(31)
                        .wrapping_add(i as u8);
                }
                let boundary = multipart_boundary(&random);
                (
                    multipart_body(&boundary, &fields),
                    Some(format!("multipart/form-data; boundary={boundary}")),
                )
            }
        };
        if let Some(ct) = content_type
            && !has(&headers, "content-type")
        {
            headers.push(("content-type".into(), ct));
        }
        if !body.is_empty() || matches!(r.method.as_str(), "POST" | "PUT" | "PATCH") && r.has_body {
            set(&mut headers, "content-length", &body.len().to_string());
        }
        let timeout = r.timeout.or(o.timeout).unwrap_or(Duration::from_secs(30));
        let mut req = HttpRequest {
            method: r.method.clone(),
            url,
            headers,
            body,
            timeout_ms: (!timeout.is_zero()).then_some(timeout.as_millis() as u64),
            ignore_https_errors: r.ignore_https_errors.unwrap_or(o.ignore_https_errors),
        };
        // `maxRedirects: 0` means "do not follow" (Playwright maps it to -1).
        let mut redirects_left: i64 = match r.max_redirects {
            Some(0) => -1,
            Some(n) => n as i64,
            None => 20,
        };
        let response = loop {
            let mut send = req.clone();
            if !has(&send.headers, "cookie") {
                let cookies = self.cookies_for(&send.url).await?;
                if !cookies.is_empty() {
                    let header = cookies
                        .iter()
                        .map(|c| format!("{}={}", c.name, c.value))
                        .collect::<Vec<_>>()
                        .join("; ");
                    send.headers.push(("cookie".into(), header));
                }
            }
            let resp = self
                .inner
                .services
                .http
                .send(send)
                .await
                .map_err(|e| Error::Request(e.to_string()))?;
            let now = self.inner.services.clock.wall_ms() / 1000.0;
            let set_cookies: Vec<NewCookie> = resp
                .headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                .filter_map(|(_, v)| parse_set_cookie(v, &resp.url, now))
                .collect();
            self.store_cookies(set_cookies).await?;
            let location = header(&resp, "location").unwrap_or_default();
            if redirects_left >= 0
                && let Some(next) = redirect_request(&req, resp.status, &location)
            {
                if redirects_left == 0 {
                    return Err(Error::Request("Max redirect count exceeded".into()));
                }
                redirects_left -= 1;
                req = next;
                continue;
            }
            if resp.status == 401
                && !has(&req.headers, "authorization")
                && header(&resp, "www-authenticate").is_some_and(|a| a.trim().starts_with("Basic"))
                && let Some(c) = self.credentials_for(&req.url)
            {
                use base64::Engine;
                let token = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", c.username, c.password));
                req.headers
                    .push(("authorization".into(), format!("Basic {token}")));
                continue;
            }
            break resp;
        };
        let fail = r.fail_on_status_code.unwrap_or(o.fail_on_status_code);
        if fail && !(200..400).contains(&response.status) {
            return Err(Error::Request(status_error(
                response.status,
                &response.status_text,
                &response.body,
            )));
        }
        Ok(ApiResponse {
            inner: Arc::new(ResponseInner {
                body: Mutex::new(Some(response.body.clone())),
                response,
            }),
        })
    }
}

fn header(r: &HttpResponse, name: &str) -> Option<String> {
    r.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

enum Body {
    None,
    Json(Value),
    Bytes(Vec<u8>),
    Form(Vec<(String, String)>),
    Multipart(Vec<(String, MultipartValue)>),
}

/// A pending request; chain options, then `.await` the [`ApiResponse`].
#[must_use = "does nothing until awaited"]
pub struct ApiRequest<'a> {
    ctx: &'a ApiRequestContext,
    method: String,
    url: String,
    params: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Body,
    has_body: bool,
    timeout: Option<Duration>,
    fail_on_status_code: Option<bool>,
    max_redirects: Option<u32>,
    ignore_https_errors: Option<bool>,
}

impl<'a> ApiRequest<'a> {
    fn new(ctx: &'a ApiRequestContext, method: &str, url: &str) -> Self {
        Self {
            ctx,
            method: method.to_owned(),
            url: url.to_owned(),
            params: Vec::new(),
            headers: Vec::new(),
            body: Body::None,
            has_body: false,
            timeout: None,
            fail_on_status_code: None,
            max_redirects: None,
            ignore_https_errors: None,
        }
    }

    /// The HTTP method (`fetch()` defaults to GET).
    pub fn method(mut self, method: &str) -> Self {
        self.method = method.to_ascii_uppercase();
        self
    }
    /// Query parameters added to the URL.
    pub fn params(mut self, params: &[(&str, &str)]) -> Self {
        self.params.extend(
            params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
        );
        self
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
    /// A JSON body (`data` with an object).
    pub fn data<T: Serialize>(mut self, data: T) -> Self {
        self.body = Body::Json(serde_json::to_value(data).unwrap_or(Value::Null));
        self.has_body = true;
        self
    }
    /// A string body (`data` with a string).
    pub fn data_text(mut self, text: &str) -> Self {
        self.body = Body::Bytes(text.as_bytes().to_vec());
        self.has_body = true;
        self
    }
    /// A binary body (`data` with a buffer).
    pub fn data_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.body = Body::Bytes(bytes);
        self.has_body = true;
        self
    }
    /// An `application/x-www-form-urlencoded` body.
    pub fn form(mut self, pairs: &[(&str, &str)]) -> Self {
        self.body = Body::Form(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        );
        self.has_body = true;
        self
    }
    /// A `multipart/form-data` body.
    pub fn multipart(mut self, fields: Vec<(&str, MultipartValue)>) -> Self {
        self.body = Body::Multipart(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
        self.has_body = true;
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    /// Fail on 4xx / 5xx responses.
    pub fn fail_on_status_code(mut self, yes: bool) -> Self {
        self.fail_on_status_code = Some(yes);
        self
    }
    /// How many redirects to follow (default 20; 0 returns the redirect).
    pub fn max_redirects(mut self, n: u32) -> Self {
        self.max_redirects = Some(n);
        self
    }
    pub fn ignore_https_errors(mut self, yes: bool) -> Self {
        self.ignore_https_errors = Some(yes);
        self
    }
}

impl<'a> IntoFuture for ApiRequest<'a> {
    type Output = Result<ApiResponse>;
    type IntoFuture = BoxFuture<'a, Result<ApiResponse>>;
    fn into_future(self) -> Self::IntoFuture {
        let ctx = self.ctx;
        async move { ctx.run(self).await }.boxed()
    }
}

struct ResponseInner {
    response: HttpResponse,
    /// `None` once disposed.
    body: Mutex<Option<Vec<u8>>>,
}

/// `APIResponse`. Cheap to clone.
#[derive(Clone)]
pub struct ApiResponse {
    inner: Arc<ResponseInner>,
}

impl std::fmt::Debug for ApiResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiResponse")
            .field("url", &self.inner.response.url)
            .field("status", &self.inner.response.status)
            .finish()
    }
}

impl ApiResponse {
    pub fn url(&self) -> &str {
        &self.inner.response.url
    }
    pub fn status(&self) -> u16 {
        self.inner.response.status
    }
    pub fn status_text(&self) -> &str {
        &self.inner.response.status_text
    }
    /// Status in 200..=299.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status())
    }
    /// Lower-cased names; repeated headers joined with `, ` (`set-cookie` with newlines).
    pub fn headers(&self) -> HashMap<String, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in &self.inner.response.headers {
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
    /// The headers as sent, in order.
    pub fn headers_array(&self) -> Vec<HeaderEntry> {
        self.inner
            .response
            .headers
            .iter()
            .map(|(name, value)| HeaderEntry {
                name: name.clone(),
                value: value.clone(),
            })
            .collect()
    }
    pub async fn body(&self) -> Result<Vec<u8>> {
        self.inner
            .body
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Error::Request("Response has been disposed".into()))
    }
    pub async fn text(&self) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.body().await?).into_owned())
    }
    pub async fn json<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(serde_json::from_slice(&self.body().await?)?)
    }
    /// `response.serverAddr()`.
    pub async fn server_addr(&self) -> Result<Option<crate::application::network::ServerAddr>> {
        Ok(self.inner.response.server_addr.as_ref().map(|(ip, port)| {
            crate::application::network::ServerAddr {
                ip_address: ip.clone(),
                port: *port,
            }
        }))
    }

    /// `response.securityDetails()`: `None` for plain HTTP.
    pub async fn security_details(
        &self,
    ) -> Result<Option<crate::application::network::SecurityDetails>> {
        let Some((protocol, cert)) = &self.inner.response.tls else {
            return Ok(None);
        };
        let summary = cert
            .as_deref()
            .and_then(crate::domain::x509::certificate_summary);
        Ok(Some(crate::application::network::SecurityDetails {
            protocol: Some(protocol.clone()),
            subject_name: summary.as_ref().and_then(|s| s.subject_name.clone()),
            issuer: summary.as_ref().and_then(|s| s.issuer.clone()),
            valid_from: summary.as_ref().map(|s| s.valid_from),
            valid_to: summary.as_ref().map(|s| s.valid_to),
        }))
    }

    /// `response.timing()`: milliseconds relative to `start_time` (-1: n/a).
    pub fn timing(&self) -> crate::domain::network::Timing {
        self.inner
            .response
            .timing
            .unwrap_or(crate::domain::network::Timing {
                start_time: 0.0,
                domain_lookup_start: -1.0,
                domain_lookup_end: -1.0,
                connect_start: -1.0,
                secure_connection_start: -1.0,
                connect_end: -1.0,
                request_start: -1.0,
                response_start: -1.0,
                response_end: -1.0,
            })
    }

    /// Frees the body; later reads fail.
    pub async fn dispose(&self) {
        *self.inner.body.lock().unwrap() = None;
    }
}
