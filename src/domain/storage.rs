//! Cookies, storage state and permissions, as Playwright models them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Cookie `SameSite` attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SameSite {
    Strict,
    #[default]
    Lax,
    None,
}

impl SameSite {
    pub fn as_str(self) -> &'static str {
        match self {
            SameSite::Strict => "Strict",
            SameSite::Lax => "Lax",
            SameSite::None => "None",
        }
    }
}

/// A cookie as `context.cookies()` returns it (and as storage state stores it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    /// Unix time in seconds; `-1` for session cookies.
    pub expires: f64,
    pub http_only: bool,
    pub secure: bool,
    pub same_site: SameSite,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<String>,
}

impl Cookie {
    /// From a protocol `Network.Cookie`; a missing `sameSite` reads as `Lax`.
    pub fn from_protocol(c: &Value) -> Self {
        let s = |k: &str| {
            c.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let b = |k: &str| c.get(k).and_then(Value::as_bool).unwrap_or(false);
        Cookie {
            name: s("name"),
            value: s("value"),
            domain: s("domain"),
            path: s("path"),
            expires: c.get("expires").and_then(Value::as_f64).unwrap_or(-1.0),
            http_only: b("httpOnly"),
            secure: b("secure"),
            same_site: match c.get("sameSite").and_then(Value::as_str) {
                Some("Strict") => SameSite::Strict,
                Some("None") => SameSite::None,
                _ => SameSite::Lax,
            },
            partition_key: c
                .pointer("/partitionKey/topLevelSite")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }

    /// The same cookie with another value.
    pub fn with_value(mut self, value: &str) -> Self {
        self.value = value.to_owned();
        self
    }

    /// As a cookie to add (to restore storage state).
    pub fn to_new_cookie(&self) -> NewCookie {
        NewCookie {
            name: self.name.clone(),
            value: self.value.clone(),
            url: None,
            domain: Some(self.domain.clone()),
            path: Some(self.path.clone()),
            expires: Some(self.expires),
            http_only: Some(self.http_only),
            secure: Some(self.secure),
            same_site: Some(self.same_site),
            partition_key: self.partition_key.clone(),
        }
    }
}

/// A cookie to add with `context.addCookies()`: give either `url`, or
/// `domain` and `path`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewCookie {
    pub name: String,
    pub value: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
    pub expires: Option<f64>,
    pub http_only: Option<bool>,
    pub secure: Option<bool>,
    pub same_site: Option<SameSite>,
    pub partition_key: Option<String>,
}

/// Playwright's `kMaxCookieExpiresDateInSeconds`.
const MAX_EXPIRES: f64 = 253_402_300_799.0;

impl NewCookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        NewCookie {
            name: name.into(),
            value: value.into(),
            ..Default::default()
        }
    }
    pub fn url(mut self, url: &str) -> Self {
        self.url = Some(url.to_owned());
        self
    }
    pub fn domain(mut self, domain: &str) -> Self {
        self.domain = Some(domain.to_owned());
        self
    }
    pub fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_owned());
        self
    }
    pub fn expires(mut self, unix_seconds: f64) -> Self {
        self.expires = Some(unix_seconds);
        self
    }
    pub fn http_only(mut self, v: bool) -> Self {
        self.http_only = Some(v);
        self
    }
    pub fn secure(mut self, v: bool) -> Self {
        self.secure = Some(v);
        self
    }
    pub fn same_site(mut self, v: SameSite) -> Self {
        self.same_site = Some(v);
        self
    }

    /// Validates like Playwright's `rewriteCookies` and fills domain, path
    /// and secure from `url`.
    pub fn resolve(&self) -> Result<NewCookie, String> {
        if self.url.is_none() && (self.domain.is_none() || self.path.is_none()) {
            return Err("Cookie should have a url or a domain/path pair".into());
        }
        if self.url.is_some() && self.domain.is_some() {
            return Err("Cookie should have either url or domain".into());
        }
        if self.url.is_some() && self.path.is_some() {
            return Err("Cookie should have either url or path".into());
        }
        if let Some(e) = self.expires
            && e != 0.0
            && ((e < 0.0 && e != -1.0) || e > MAX_EXPIRES)
        {
            return Err("Cookie should have a valid expires, only -1 or a positive number for the unix timestamp in seconds is allowed".into());
        }
        let mut copy = self.clone();
        if let Some(url) = &self.url {
            if url == "about:blank" {
                return Err(format!("Blank page can not have cookie \"{}\"", self.name));
            }
            if url.starts_with("data:") {
                return Err(format!(
                    "Data URL page can not have cookie \"{}\"",
                    self.name
                ));
            }
            let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL {url}: {e}"))?;
            let pathname = parsed.path();
            copy.domain = Some(parsed.host_str().unwrap_or_default().to_owned());
            copy.path = Some(pathname[..pathname.rfind('/').map_or(0, |i| i + 1)].to_owned());
            copy.secure = Some(parsed.scheme() == "https");
        }
        Ok(copy)
    }

    /// Parameters for `Storage.setCookies` (after [`NewCookie::resolve`]).
    pub fn to_protocol(&self) -> Value {
        let mut v = json!({"name": self.name, "value": self.value});
        if let Some(u) = &self.url {
            v["url"] = json!(u);
        }
        if let Some(d) = &self.domain {
            v["domain"] = json!(d);
        }
        if let Some(p) = &self.path {
            v["path"] = json!(p);
        }
        if let Some(e) = self.expires {
            v["expires"] = json!(e);
        }
        if let Some(h) = self.http_only {
            v["httpOnly"] = json!(h);
        }
        if let Some(s) = self.secure {
            v["secure"] = json!(s);
        }
        if let Some(s) = self.same_site {
            v["sameSite"] = json!(s.as_str());
        }
        if let Some(k) = &self.partition_key {
            v["partitionKey"] = json!({"topLevelSite": k, "hasCrossSiteAncestor": true});
        }
        v
    }
}

fn is_local_hostname(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "[::1]"
        || host.strip_prefix("127.").is_some_and(|rest| {
            let parts: Vec<&str> = rest.split('.').collect();
            parts.len() == 3
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
}

/// RFC 6265 domain matching (Playwright 1.65's `domainMatches`): a
/// host-only cookie matches its host alone.
pub fn domain_matches(hostname: &str, cookie_domain: &str) -> bool {
    hostname == cookie_domain
        || (cookie_domain.starts_with('.') && format!(".{hostname}").ends_with(cookie_domain))
}

/// RFC 6265 path matching (Playwright 1.65's `pathMatches`).
pub fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

/// Playwright's `cookieMatchesURL`.
fn cookie_matches_url(c: &Cookie, u: &url::Url) -> bool {
    let host = match u.host() {
        Some(url::Host::Ipv6(a)) => format!("[{a}]"),
        Some(h) => h.to_string(),
        None => String::new(),
    };
    domain_matches(&host, &c.domain)
        && path_matches(u.path(), &c.path)
        && !(c.secure && u.scheme() != "https" && !is_local_hostname(&host))
}

/// Playwright's `filterCookies`: cookies sent to any of `urls` (all when empty).
pub fn filter_cookies(cookies: Vec<Cookie>, urls: &[&str]) -> Vec<Cookie> {
    let parsed: Vec<url::Url> = urls
        .iter()
        .filter_map(|u| url::Url::parse(u).ok())
        .collect();
    if urls.is_empty() {
        return cookies;
    }
    cookies
        .into_iter()
        .filter(|c| parsed.iter().any(|u| cookie_matches_url(c, u)))
        .collect()
}

/// A `localStorage` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameValue {
    pub name: String,
    pub value: String,
}

/// Storage of one origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OriginState {
    pub origin: String,
    #[serde(default)]
    pub local_storage: Vec<NameValue>,
    /// Databases, as Playwright's storage script serialises them (when asked for).
    #[serde(default, rename = "indexedDB", skip_serializing_if = "Option::is_none")]
    pub indexed_db: Option<Vec<serde_json::Value>>,
    /// Origin private file system entries (`{ path, type, base64 }`), as
    /// Playwright's storage script serialises them (when asked for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opfs: Option<Vec<serde_json::Value>>,
}

/// Playwright's storage state (`context.storageState()`), JSON-compatible
/// with files Playwright writes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StorageState {
    #[serde(default)]
    pub cookies: Vec<Cookie>,
    #[serde(default)]
    pub origins: Vec<OriginState>,
}

/// Filter for `context.clearCookies(options)`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClearCookies {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub path: Option<String>,
}

impl ClearCookies {
    pub fn name(mut self, v: &str) -> Self {
        self.name = Some(v.to_owned());
        self
    }
    pub fn domain(mut self, v: &str) -> Self {
        self.domain = Some(v.to_owned());
        self
    }
    pub fn path(mut self, v: &str) -> Self {
        self.path = Some(v.to_owned());
        self
    }
    /// Whether `cookie` is to be cleared.
    pub fn matches(&self, cookie: &Cookie) -> bool {
        self.name.as_ref().is_none_or(|n| *n == cookie.name)
            && self.domain.as_ref().is_none_or(|d| *d == cookie.domain)
            && self.path.as_ref().is_none_or(|p| *p == cookie.path)
    }
}

/// Maps web permission names to Chromium's, like Playwright.
pub fn protocol_permissions(permissions: &[&str]) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    for p in permissions {
        let mapped: &[&'static str] = match *p {
            "geolocation" => &["geolocation"],
            "midi" => &["midi"],
            "notifications" => &["notifications"],
            "camera" => &["videoCapture"],
            "microphone" => &["audioCapture"],
            "background-sync" => &["backgroundSync"],
            "ambient-light-sensor" | "accelerometer" | "gyroscope" | "magnetometer" => &["sensors"],
            "clipboard-read" => &["clipboardReadWrite"],
            "clipboard-write" => &["clipboardSanitizedWrite"],
            "payment-handler" => &["paymentHandler"],
            "midi-sysex" => &["midiSysex"],
            "storage-access" => &["storageAccess"],
            "local-fonts" => &["localFonts"],
            "local-network-access" => &["localNetworkAccess", "localNetwork", "loopbackNetwork"],
            "screen-wake-lock" => &["wakeLockScreen"],
            other => return Err(format!("Unknown permission: {other}")),
        };
        out.extend_from_slice(mapped);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cookie(name: &str, domain: &str, path: &str, secure: bool) -> Cookie {
        Cookie {
            name: name.into(),
            value: "v".into(),
            domain: domain.into(),
            path: path.into(),
            expires: -1.0,
            http_only: false,
            secure,
            same_site: SameSite::Lax,
            partition_key: None,
        }
    }

    #[test]
    fn protocol_cookies_default_to_lax() {
        let c = Cookie::from_protocol(&json!({
            "name": "a", "value": "1", "domain": "example.com", "path": "/",
            "expires": -1, "size": 2, "httpOnly": true, "secure": false, "session": true
        }));
        assert_eq!(
            c,
            Cookie {
                http_only: true,
                ..cookie("a", "example.com", "/", false)
            }
            .with_value("1")
        );
        let strict = Cookie::from_protocol(
            &json!({"name": "b", "value": "", "domain": "x", "path": "/",
            "expires": 2000000000.5, "httpOnly": false, "secure": true, "sameSite": "Strict"}),
        );
        assert_eq!(strict.same_site, SameSite::Strict);
        assert_eq!(strict.expires, 2000000000.5);
    }

    #[test]
    fn cookies_serialize_like_playwright_storage_state() {
        let c = cookie("a", "127.0.0.1", "/", false);
        assert_eq!(
            serde_json::to_value(&c).unwrap(),
            json!({"name": "a", "value": "v", "domain": "127.0.0.1", "path": "/", "expires": -1.0,
                   "httpOnly": false, "secure": false, "sameSite": "Lax"})
        );
        let state = StorageState {
            cookies: vec![c],
            origins: vec![OriginState {
                opfs: None,
                indexed_db: None,
                origin: "http://127.0.0.1:1".into(),
                local_storage: vec![NameValue {
                    name: "k".into(),
                    value: "v".into(),
                }],
            }],
        };
        let text = serde_json::to_string(&state).unwrap();
        assert!(text.contains("\"localStorage\":[{\"name\":\"k\",\"value\":\"v\"}]"));
        let back: StorageState = serde_json::from_str(&text).unwrap();
        assert_eq!(back, state);
        // Files written by Playwright parse too (origins may hold more keys).
        let pw: StorageState = serde_json::from_value(json!({
            "cookies": [],
            "origins": [{"origin": "https://a.test", "localStorage": [], "indexedDB": []}]
        }))
        .unwrap();
        assert_eq!(pw.origins[0].origin, "https://a.test");
    }

    #[test]
    fn filter_cookies_by_url_like_playwright() {
        let all = vec![
            cookie("root", "127.0.0.1", "/", false),
            cookie("sub", "127.0.0.1", "/sub", false),
            cookie("dot", ".example.com", "/", false),
            cookie("secure", "example.com", "/", true),
        ];
        let names = |urls: &[&str]| -> Vec<String> {
            filter_cookies(all.clone(), urls)
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names(&[]).len(), 4);
        assert_eq!(names(&["http://127.0.0.1:8080/"]), ["root"]);
        assert_eq!(names(&["http://127.0.0.1:8080/sub/x"]), ["root", "sub"]);
        assert_eq!(names(&["http://www.example.com/"]), ["dot"]);
        assert_eq!(names(&["https://example.com/"]), ["dot", "secure"]);
        assert_eq!(names(&["http://other.test/"]), Vec::<String>::new());
    }

    /// Playwright 1.65 matches cookies to URLs like RFC 6265.
    #[test]
    fn filter_cookies_follows_rfc_6265() {
        let all = vec![
            cookie("host_only", "example.com", "/", false),
            cookie("api", "example.com", "/api", false),
            cookie("foo_dir", "example.com", "/foo/", false),
            cookie("secure_local", "127.0.0.1", "/", true),
            cookie("secure_v6", "[::1]", "/", true),
        ];
        let names = |url: &str| -> Vec<String> {
            filter_cookies(all.clone(), &[url])
                .into_iter()
                .map(|c| c.name)
                .collect()
        };
        assert_eq!(names("http://www.example.com/api"), Vec::<String>::new());
        assert_eq!(names("http://example.com/apiv2"), ["host_only"]);
        assert_eq!(names("http://example.com/api"), ["host_only", "api"]);
        assert_eq!(names("http://example.com/api/x"), ["host_only", "api"]);
        assert_eq!(names("http://example.com/foo"), ["host_only"]);
        assert_eq!(
            names("http://example.com/foo/bar"),
            ["host_only", "foo_dir"]
        );
        assert_eq!(names("http://127.0.0.1:9/"), ["secure_local"]);
        assert_eq!(names("http://[::1]:9/"), ["secure_v6"]);
    }

    #[test]
    fn new_cookies_from_a_url() {
        let c = NewCookie::new("a", "1").url("https://example.com/dir/page.html");
        let r = c.resolve().unwrap();
        assert_eq!(
            (r.domain.as_deref(), r.path.as_deref(), r.secure),
            (Some("example.com"), Some("/dir/"), Some(true))
        );
        let err = |c: NewCookie| c.resolve().unwrap_err();
        assert_eq!(
            err(NewCookie::new("a", "1")),
            "Cookie should have a url or a domain/path pair"
        );
        assert_eq!(
            err(NewCookie::new("a", "1").url("http://x/").domain("x")),
            "Cookie should have either url or domain"
        );
        assert_eq!(
            err(NewCookie::new("a", "1").url("http://x/").path("/")),
            "Cookie should have either url or path"
        );
        assert_eq!(
            err(NewCookie::new("a", "1").url("about:blank")),
            "Blank page can not have cookie \"a\""
        );
        assert_eq!(
            err(NewCookie::new("a", "1").domain("x").path("/").expires(-5.0)),
            "Cookie should have a valid expires, only -1 or a positive number for the unix timestamp in seconds is allowed"
        );
        let ok = NewCookie::new("a", "1").domain("x").path("/").expires(-1.0);
        assert!(ok.resolve().is_ok());
    }

    #[test]
    fn new_cookies_to_protocol() {
        let c = NewCookie::new("a", "1")
            .url("http://example.com/")
            .http_only(true)
            .same_site(SameSite::Strict)
            .resolve()
            .unwrap();
        assert_eq!(
            c.to_protocol(),
            json!({"name": "a", "value": "1", "url": "http://example.com/", "domain": "example.com",
                   "path": "/", "secure": false, "httpOnly": true, "sameSite": "Strict"})
        );
    }

    #[test]
    fn web_permissions_map_to_protocol() {
        assert_eq!(
            protocol_permissions(&["geolocation"]).unwrap(),
            ["geolocation"]
        );
        assert_eq!(
            protocol_permissions(&["camera", "microphone", "clipboard-read"]).unwrap(),
            ["videoCapture", "audioCapture", "clipboardReadWrite"]
        );
        assert_eq!(
            protocol_permissions(&["local-network-access"]).unwrap(),
            ["localNetworkAccess", "localNetwork", "loopbackNetwork"]
        );
        assert_eq!(
            protocol_permissions(&["bogus"]).unwrap_err(),
            "Unknown permission: bogus"
        );
    }

    #[test]
    fn clear_cookie_filters() {
        let c = cookie("a", "example.com", "/p", false);
        assert!(ClearCookies::default().matches(&c));
        assert!(ClearCookies::default().name("a").matches(&c));
        assert!(!ClearCookies::default().name("b").matches(&c));
        assert!(
            ClearCookies::default()
                .domain("example.com")
                .path("/p")
                .matches(&c)
        );
        assert!(!ClearCookies::default().path("/").matches(&c));
    }
}
