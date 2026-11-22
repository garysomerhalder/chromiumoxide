//! HAR 1.2 (HTTP Archive) as Playwright records and replays it.

use std::path::PathBuf;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::domain::network::Timing;

/// `recordHar` context option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordHar {
    /// Where the HAR is written when the context closes.
    pub path: PathBuf,
    /// Leave response bodies out (`content: 'omit'`).
    pub omit_content: bool,
    /// Only record URLs matching this glob.
    pub url_filter: Option<String>,
}

impl RecordHar {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        RecordHar {
            path: path.into(),
            omit_content: false,
            url_filter: None,
        }
    }
}

/// What `routeFromHAR` does with requests the HAR has no entry for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HarNotFound {
    /// Fail the request (Playwright's default).
    #[default]
    Abort,
    /// Send it on to the next route handler / the network.
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Har {
    pub log: Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Creator {
    pub name: String,
    pub version: String,
}

impl Default for Creator {
    fn default() -> Self {
        Creator {
            name: "chromiumoxide".into(),
            version: "0.10.0".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Log {
    pub version: String,
    pub creator: Creator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<Creator>,
    #[serde(default)]
    pub pages: Vec<HarPage>,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Default for Log {
    fn default() -> Self {
        Log {
            version: "1.2".into(),
            creator: Creator::default(),
            browser: None,
            pages: Vec::new(),
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarPage {
    pub started_date_time: String,
    pub id: String,
    pub title: String,
    pub page_timings: PageTimings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTimings {
    pub on_content_load: f64,
    pub on_load: f64,
}

impl Default for PageTimings {
    fn default() -> Self {
        PageTimings {
            on_content_load: -1.0,
            on_load: -1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NameValuePair {
    pub name: String,
    pub value: String,
}

impl NameValuePair {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        NameValuePair {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pageref: Option<String>,
    pub started_date_time: String,
    pub time: f64,
    pub request: HarRequest,
    pub response: HarResponse,
    #[serde(default)]
    pub cache: serde_json::Map<String, serde_json::Value>,
    pub timings: Timings,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "serverIPAddress"
    )]
    pub server_ip_address: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "_serverPort"
    )]
    pub server_port: Option<u16>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "_resourceType"
    )]
    pub resource_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarRequest {
    pub method: String,
    pub url: String,
    pub http_version: String,
    #[serde(default)]
    pub cookies: Vec<NameValuePair>,
    #[serde(default)]
    pub headers: Vec<NameValuePair>,
    #[serde(default)]
    pub query_string: Vec<NameValuePair>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_data: Option<PostData>,
    pub headers_size: i64,
    pub body_size: i64,
}

impl Default for HarRequest {
    fn default() -> Self {
        HarRequest {
            method: "GET".into(),
            url: String::new(),
            http_version: "HTTP/1.1".into(),
            cookies: Vec::new(),
            headers: Vec::new(),
            query_string: Vec::new(),
            post_data: None,
            headers_size: -1,
            body_size: -1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostData {
    pub mime_type: String,
    pub text: String,
    #[serde(default)]
    pub params: Vec<NameValuePair>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarResponse {
    pub status: i64,
    pub status_text: String,
    pub http_version: String,
    #[serde(default)]
    pub cookies: Vec<NameValuePair>,
    #[serde(default)]
    pub headers: Vec<NameValuePair>,
    pub content: Content,
    #[serde(rename = "redirectURL", default)]
    pub redirect_url: String,
    pub headers_size: i64,
    pub body_size: i64,
}

impl Default for HarResponse {
    fn default() -> Self {
        HarResponse {
            status: -1,
            status_text: String::new(),
            http_version: "HTTP/1.1".into(),
            cookies: Vec::new(),
            headers: Vec::new(),
            content: Content::default(),
            redirect_url: String::new(),
            headers_size: -1,
            body_size: -1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Content {
    pub size: i64,
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// The body stored next to the trace (`resources/<sha1>.<ext>`).
    #[serde(default, rename = "_file", skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Default for Content {
    fn default() -> Self {
        Content {
            size: -1,
            mime_type: "x-unknown".into(),
            text: None,
            encoding: None,
            file: None,
        }
    }
}

impl Content {
    /// Embedded body: text as is, anything else base64.
    pub fn from_body(body: &[u8], mime_type: &str) -> Self {
        let text_ok = is_text_mime(mime_type) && std::str::from_utf8(body).is_ok();
        Content {
            size: body.len() as i64,
            mime_type: mime_type.to_owned(),
            text: Some(if text_ok {
                String::from_utf8_lossy(body).into_owned()
            } else {
                base64::engine::general_purpose::STANDARD.encode(body)
            }),
            encoding: (!text_ok).then(|| "base64".to_owned()),
            file: None,
        }
    }

    /// The body bytes (empty when not recorded).
    pub fn body(&self) -> Vec<u8> {
        let text = self.text.as_deref().unwrap_or_default();
        if self.encoding.as_deref() == Some("base64") {
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .unwrap_or_default()
        } else {
            text.as_bytes().to_vec()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timings {
    #[serde(default = "minus_one")]
    pub dns: f64,
    #[serde(default = "minus_one")]
    pub connect: f64,
    #[serde(default = "minus_one")]
    pub ssl: f64,
    pub send: f64,
    pub wait: f64,
    pub receive: f64,
}

fn minus_one() -> f64 {
    -1.0
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            dns: -1.0,
            connect: -1.0,
            ssl: -1.0,
            send: 0.0,
            wait: -1.0,
            receive: -1.0,
        }
    }
}

impl Timings {
    /// The entry's `time`: the sum of the known phases.
    pub fn total(&self) -> f64 {
        [self.dns, self.connect, self.send, self.wait, self.receive]
            .iter()
            .filter(|v| **v >= 0.0)
            .sum()
    }
}

fn span(start: f64, end: f64) -> f64 {
    if start >= 0.0 && end >= 0.0 {
        end - start
    } else {
        -1.0
    }
}

/// HAR timings from a request's `timing()`.
pub fn timings(t: &Timing) -> Timings {
    Timings {
        dns: span(t.domain_lookup_start, t.domain_lookup_end),
        connect: span(t.connect_start, t.connect_end),
        ssl: span(t.secure_connection_start, t.connect_end),
        send: 0.0,
        wait: span(t.request_start, t.response_start),
        receive: span(t.response_start, t.response_end),
    }
}

/// ISO 8601 UTC time with milliseconds, from Unix milliseconds.
pub fn iso_time(epoch_ms: f64) -> String {
    let ms = epoch_ms.max(0.0).round() as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Whether a body of this type is stored as text.
pub fn is_text_mime(mime: &str) -> bool {
    let m = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    m.starts_with("text/")
        || m.ends_with("+xml")
        || m.ends_with("+json")
        || matches!(
            m.as_str(),
            "application/json"
                | "application/javascript"
                | "application/xml"
                | "application/x-www-form-urlencoded"
                | "application/ecmascript"
        )
}

fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default()
}

/// The URL's query as name/value pairs.
pub fn query_string(url: &str) -> Vec<NameValuePair> {
    let Some((_, q)) = url.split_once('?') else {
        return Vec::new();
    };
    let q = q.split('#').next().unwrap_or("");
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            NameValuePair::new(decode(k), decode(v))
        })
        .collect()
}

/// A `Cookie` request header as pairs.
pub fn request_cookies(header: &str) -> Vec<NameValuePair> {
    header
        .split(';')
        .filter_map(|c| {
            let (k, v) = c.trim().split_once('=')?;
            Some(NameValuePair::new(k.trim(), v.trim()))
        })
        .collect()
}

/// The name and value of a `Set-Cookie` line.
pub fn response_cookie(line: &str) -> Option<NameValuePair> {
    let first = line.split(';').next()?;
    let (k, v) = first.split_once('=')?;
    Some(NameValuePair::new(k.trim(), v.trim()))
}

impl Log {
    /// The entry to replay for a request (Playwright's lookup): same method
    /// and URL, preferring the same post data; redirects are followed.
    pub fn find(&self, method: &str, url: &str, post_data: Option<&[u8]>) -> Option<&Entry> {
        let mut url = url.to_owned();
        for _ in 0..20 {
            let candidates: Vec<&Entry> = self
                .entries
                .iter()
                .filter(|e| e.request.url == url && e.request.method.eq_ignore_ascii_case(method))
                .collect();
            let entry = candidates
                .iter()
                .find(|e| {
                    post_data.is_some_and(|p| {
                        e.request
                            .post_data
                            .as_ref()
                            .is_some_and(|d| d.text.as_bytes() == p)
                    })
                })
                .or_else(|| candidates.first())
                .copied()?;
            let redirect = (300..400).contains(&entry.response.status)
                && !entry.response.redirect_url.is_empty();
            if !redirect {
                return Some(entry);
            }
            url = url::Url::parse(&url)
                .and_then(|base| base.join(&entry.response.redirect_url))
                .map(|u| u.to_string())
                .unwrap_or_else(|_| entry.response.redirect_url.clone());
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_times() {
        assert_eq!(iso_time(0.0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_time(1_795_000_000_123.0), "2026-11-18T11:06:40.123Z");
    }

    #[test]
    fn text_mime_types() {
        assert!(is_text_mime("text/html; charset=utf-8"));
        assert!(is_text_mime("application/json"));
        assert!(is_text_mime("application/javascript"));
        assert!(is_text_mime("image/svg+xml"));
        assert!(!is_text_mime("image/png"));
        assert!(!is_text_mime("application/octet-stream"));
    }

    #[test]
    fn query_strings_and_cookies() {
        assert_eq!(
            query_string("http://x/p?a=1&b=two%20words&c"),
            [
                NameValuePair::new("a", "1"),
                NameValuePair::new("b", "two words"),
                NameValuePair::new("c", "")
            ]
        );
        assert!(query_string("http://x/p").is_empty());
        assert_eq!(
            request_cookies("a=1; b=2"),
            [NameValuePair::new("a", "1"), NameValuePair::new("b", "2")]
        );
        assert_eq!(
            response_cookie("sid=abc; Path=/; HttpOnly"),
            Some(NameValuePair::new("sid", "abc"))
        );
    }

    #[test]
    fn timings_from_request_timing() {
        let t = timings(&crate::domain::network::Timing {
            start_time: 0.0,
            domain_lookup_start: 1.0,
            domain_lookup_end: 3.0,
            connect_start: 3.0,
            secure_connection_start: -1.0,
            connect_end: 5.0,
            request_start: 5.0,
            response_start: 9.0,
            response_end: 12.0,
        });
        assert_eq!(
            (t.dns, t.connect, t.ssl, t.send, t.wait, t.receive),
            (2.0, 2.0, -1.0, 0.0, 4.0, 3.0)
        );
        assert_eq!(t.total(), 11.0);
    }

    fn method_url(
        method: &str,
        url: &str,
        post: Option<&str>,
        status: u16,
        redirect: &str,
    ) -> Entry {
        let mut e = Entry::default();
        e.request.method = method.into();
        e.request.url = url.into();
        e.request.post_data = post.map(|t| PostData {
            mime_type: "text/plain".into(),
            text: t.into(),
            params: Vec::new(),
        });
        e.response.status = status as i64;
        e.response.redirect_url = redirect.into();
        e
    }

    #[test]
    fn replay_lookup_like_playwright() {
        let log = Log {
            entries: vec![
                method_url("GET", "http://x/a", None, 302, "http://x/b"),
                method_url("GET", "http://x/b", None, 200, ""),
                method_url("POST", "http://x/api", Some("one"), 200, ""),
                method_url("POST", "http://x/api", Some("two"), 201, ""),
            ],
            ..Default::default()
        };
        assert_eq!(
            log.find("GET", "http://x/a", None).unwrap().request.url,
            "http://x/b"
        );
        assert_eq!(
            log.find("POST", "http://x/api", Some(b"two"))
                .unwrap()
                .response
                .status,
            201
        );
        assert_eq!(
            log.find("POST", "http://x/api", Some(b"zzz"))
                .unwrap()
                .response
                .status,
            200
        );
        assert!(log.find("GET", "http://x/api", None).is_none());
        assert!(log.find("GET", "http://x/none", None).is_none());
    }

    #[test]
    fn content_body_round_trip() {
        let text = Content::from_body(b"<p>x</p>", "text/html");
        assert_eq!(
            (text.size, text.text.as_deref(), text.encoding.as_deref()),
            (8, Some("<p>x</p>"), None)
        );
        let bin = Content::from_body(&[0, 1, 255], "image/png");
        assert_eq!(bin.encoding.as_deref(), Some("base64"));
        assert_eq!(bin.body(), vec![0, 1, 255]);
        assert_eq!(text.body(), b"<p>x</p>");
    }

    #[test]
    fn serializes_like_playwright() {
        let mut e = method_url("GET", "http://x/", None, 200, "");
        e.started_date_time = iso_time(0.0);
        let v = serde_json::to_value(&e).unwrap();
        for key in [
            "startedDateTime",
            "time",
            "request",
            "response",
            "cache",
            "timings",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert!(v["request"].get("queryString").is_some());
        assert!(v["response"].get("redirectURL").is_some());
        assert!(v["request"].get("postData").is_none());
    }
}
