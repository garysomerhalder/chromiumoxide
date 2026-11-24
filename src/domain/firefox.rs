//! Firefox: launch arguments and WebDriver BiDi value conversion (pure).

use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value};

use crate::domain::emulation::{ColorScheme, ContextOptions, ServiceWorkers};
use crate::domain::har::{
    Content, Entry, HarRequest, HarResponse, NameValuePair, PostData, Timings, iso_time,
    query_string, request_cookies, response_cookie,
};
use crate::domain::storage::{Cookie, NewCookie, SameSite};

/// Playwright's Firefox arguments with the WebDriver BiDi endpoint instead
/// of Juggler's pipe. Playwright passes `-silent` (no window) because its
/// Juggler agent keeps Firefox alive; over BiDi Firefox quits without a
/// window, so an `about:blank` window is opened instead.
pub fn command_line(profile: &Path, headless: bool, args: &[String]) -> Vec<String> {
    let mut out = vec!["-no-remote".to_owned()];
    if headless {
        out.push("-headless".to_owned());
    } else {
        out.push("-wait-for-browser".to_owned());
        out.push("-foreground".to_owned());
    }
    out.push("-profile".to_owned());
    out.push(profile.display().to_string());
    out.push("--remote-debugging-port=0".to_owned());
    out.extend(args.iter().cloned());
    out.push("about:blank".to_owned());
    out
}

/// Files written into a new profile before launch, as `(relative path,
/// contents)`. In headless mode Playwright's Juggler hides every scrollbar
/// with an agent stylesheet (`* { scrollbar-width: none !important }`, its
/// `hidden-scrollbars.css`), so layouts and screenshots have no scrollbar
/// gutter. Over BiDi the same rule goes in the profile's `userContent.css`
/// (a user stylesheet, enabled by a pref): invisible to pages, like Juggler's.
pub fn profile_files(headless: bool) -> Vec<(&'static str, &'static str)> {
    if !headless {
        return Vec::new();
    }
    vec![
        (
            "user.js",
            "user_pref(\"toolkit.legacyUserProfileCustomizations.stylesheets\", true);\n",
        ),
        (
            "chrome/userContent.css",
            "* {\n  scrollbar-width: none !important;\n}\n",
        ),
    ]
}

/// The WebDriver `input.performActions` value of a Playwright key name
/// (`"a"`, `"KeyA"`, `"Backspace"`, `"Shift"`, ...): printable keys are
/// their character, the others WebDriver's private-use code points.
pub fn webdriver_key(name: &str) -> Option<String> {
    let key = crate::domain::input::key_description(name)
        .map(|d| d.key)
        .unwrap_or_else(|| name.to_owned());
    if key.chars().count() == 1 {
        return Some(key);
    }
    let code = match key.as_str() {
        "Cancel" => '\u{E001}',
        "Help" => '\u{E002}',
        "Backspace" => '\u{E003}',
        "Tab" => '\u{E004}',
        "Clear" => '\u{E005}',
        "Enter" => '\u{E007}',
        "Shift" => '\u{E008}',
        "Control" => '\u{E009}',
        "Alt" => '\u{E00A}',
        "Pause" => '\u{E00B}',
        "Escape" => '\u{E00C}',
        "PageUp" => '\u{E00E}',
        "PageDown" => '\u{E00F}',
        "End" => '\u{E010}',
        "Home" => '\u{E011}',
        "ArrowLeft" => '\u{E012}',
        "ArrowUp" => '\u{E013}',
        "ArrowRight" => '\u{E014}',
        "ArrowDown" => '\u{E015}',
        "Insert" => '\u{E016}',
        "Delete" => '\u{E017}',
        "Meta" => '\u{E03D}',
        f if f.starts_with('F') && f[1..].parse::<u32>().is_ok_and(|n| (1..=12).contains(&n)) => {
            char::from_u32(0xE031 + f[1..].parse::<u32>().unwrap() - 1)?
        }
        _ => return None,
    };
    Some(code.to_string())
}

/// A BiDi `network.Cookie` as Playwright's cookie (`expiry` absent: a
/// session cookie, `-1`; BiDi's `default` same-site reads as `Lax`, like
/// Chromium's missing value).
pub fn cookie_from_bidi(c: &Value) -> Cookie {
    let s = |k: &str| {
        c.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Cookie {
        name: s("name"),
        value: c
            .pointer("/value/value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        domain: s("domain"),
        path: s("path"),
        expires: c.get("expiry").and_then(Value::as_f64).unwrap_or(-1.0),
        http_only: c.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
        secure: c.get("secure").and_then(Value::as_bool).unwrap_or(false),
        same_site: match c.get("sameSite").and_then(Value::as_str) {
            Some("strict") => SameSite::Strict,
            Some("none") => SameSite::None,
            _ => SameSite::Lax,
        },
        partition_key: None,
    }
}

/// `storage.setCookie`'s `cookie` for a cookie already checked and filled
/// by [`NewCookie::resolve`].
pub fn cookie_to_bidi(c: &NewCookie) -> Value {
    let mut v = serde_json::json!({
        "name": c.name,
        "value": {"type": "string", "value": c.value},
        "domain": c.domain.clone().unwrap_or_default(),
        "path": c.path.clone().unwrap_or_else(|| "/".to_owned()),
    });
    if let Some(h) = c.http_only {
        v["httpOnly"] = Value::Bool(h);
    }
    if let Some(sec) = c.secure {
        v["secure"] = Value::Bool(sec);
    }
    if let Some(ss) = c.same_site {
        v["sameSite"] = Value::String(
            match ss {
                SameSite::Strict => "strict",
                SameSite::Lax => "lax",
                SameSite::None => "none",
            }
            .to_owned(),
        );
    }
    if let Some(e) = c.expires.filter(|e| *e > 0.0) {
        v["expiry"] = Value::Number(Number::from(e as u64));
    }
    v
}

/// A BiDi preload script (a function) that runs Playwright's init script
/// source as a script: through global `eval`, so top-level `var` and
/// function declarations become globals, like Chromium's
/// `Page.addScriptToEvaluateOnNewDocument`.
pub fn init_script_function(source: &str) -> String {
    format!(
        "() => {{ (0, eval)({}); }}",
        Value::String(source.to_owned())
    )
}

/// Context options Firefox 159 cannot honour over WebDriver BiDi (no
/// command for them), by their Playwright names. Creating a context with any
/// of them set is an error rather than a silent no-op.
pub fn unsupported_context_options(o: &ContextOptions) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut check = |set: bool, name: &'static str| {
        if set {
            out.push(name);
        }
    };
    check(o.is_mobile, "isMobile");
    check(o.has_touch, "hasTouch");
    check(
        matches!(
            o.color_scheme,
            Some(ColorScheme::Dark | ColorScheme::NoPreference)
        ),
        "colorScheme",
    );
    check(o.reduced_motion.is_some(), "reducedMotion");
    check(o.forced_colors.is_some(), "forcedColors");
    check(o.contrast.is_some(), "contrast");
    check(o.proxy.is_some(), "proxy");
    check(!o.client_certificates.is_empty(), "clientCertificates");
    check(!o.java_script_enabled, "javaScriptEnabled");
    check(o.bypass_csp, "bypassCSP");
    check(o.service_workers != ServiceWorkers::Allow, "serviceWorkers");
    out
}

/// Playwright's resource type of a request, from BiDi's `destination` and
/// `initiatorType` (Fetch's request destination and initiator).
pub fn resource_type(destination: &str, initiator_type: Option<&str>) -> &'static str {
    match destination {
        "document" | "iframe" | "frame" => "document",
        "style" => "stylesheet",
        "script" | "worker" | "sharedworker" | "serviceworker" | "audioworklet"
        | "paintworklet" => "script",
        "image" => "image",
        "font" => "font",
        "audio" | "video" => "media",
        "track" => "texttrack",
        "manifest" => "manifest",
        _ => match initiator_type {
            Some("fetch") => "fetch",
            Some("xmlhttprequest") => "xhr",
            Some("beacon") => "ping",
            Some("eventsource") => "eventsource",
            _ => "other",
        },
    }
}

/// Playwright's `request.timing()` from BiDi `network.FetchTimingInfo`
/// (epoch milliseconds, 0 when unknown): `start_time` is the request time,
/// the others are relative to it, `-1` when unknown.
pub fn timing_from_bidi(t: &Value) -> crate::domain::network::Timing {
    let start = t["requestTime"].as_f64().unwrap_or_default();
    let rel = |k: &str| match t[k].as_f64() {
        Some(v) if v > 0.0 && start > 0.0 => v - start,
        _ => -1.0,
    };
    crate::domain::network::Timing {
        start_time: start,
        domain_lookup_start: rel("dnsStart"),
        domain_lookup_end: rel("dnsEnd"),
        connect_start: rel("connectStart"),
        secure_connection_start: rel("tlsStart"),
        connect_end: rel("connectEnd"),
        request_start: rel("requestStart"),
        response_start: rel("responseStart"),
        response_end: rel("responseEnd"),
    }
}

/// Playwright's `response.httpVersion()` from BiDi's `protocol`.
pub fn http_version_of(protocol: &str) -> String {
    match protocol.to_ascii_lowercase().as_str() {
        "h2" | "http/2" | "http/2.0" => "HTTP/2.0".into(),
        "h3" | "http/3" | "http/3.0" => "HTTP/3.0".into(),
        "" => "HTTP/1.1".into(),
        p => p.to_ascii_uppercase(),
    }
}

/// The origin (`scheme://host[:port]`) of an http(s) URL, for BiDi
/// permissions; `None` for other URLs.
pub fn origin_of(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    matches!(u.scheme(), "http" | "https").then(|| u.origin().ascii_serialization())
}

/// A BiDi `script.RemoteValue` as JSON, the way Playwright's `evaluate`
/// returns values: `undefined` is `null`, special numbers become `null`,
/// dates their ISO string, maps and objects objects, sets and arrays arrays,
/// other platform objects `{}`.
pub fn remote_value_to_json(v: &Value) -> Value {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    let value = v.get("value");
    match kind {
        "undefined" | "null" => Value::Null,
        "string" => value.cloned().unwrap_or(Value::Null),
        "boolean" => value.cloned().unwrap_or(Value::Null),
        "number" => match value {
            Some(Value::Number(n)) => Value::Number(n.clone()),
            Some(Value::String(s)) if s == "-0" => Value::Number(Number::from(0)),
            _ => Value::Null,
        },
        "bigint" => value
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok())
            .map(|n| Value::Number(n.into()))
            .unwrap_or(Value::Null),
        "date" => value.cloned().unwrap_or(Value::Null),
        "array" | "set" => Value::Array(
            value
                .and_then(Value::as_array)
                .map(|items| items.iter().map(remote_value_to_json).collect())
                .unwrap_or_default(),
        ),
        "object" | "map" => {
            let mut map = Map::new();
            for pair in value.and_then(Value::as_array).into_iter().flatten() {
                let (Some(k), Some(val)) = (pair.get(0), pair.get(1)) else {
                    continue;
                };
                let key = match k {
                    Value::String(s) => s.clone(),
                    other => remote_value_to_json(other).to_string(),
                };
                map.insert(key, remote_value_to_json(val));
            }
            Value::Object(map)
        }
        _ => Value::Object(Map::new()),
    }
}

/// The BiDi `network.Header` list as HAR pairs (byte values decoded).
fn har_headers(headers: &Value) -> Vec<NameValuePair> {
    headers
        .as_array()
        .map(|hs| {
            hs.iter()
                .map(|h| {
                    NameValuePair::new(
                        h["name"].as_str().unwrap_or_default(),
                        h.pointer("/value/value")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

fn header<'a>(headers: &'a [NameValuePair], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// Playwright's HAR `httpVersion` for a network protocol name.
fn http_version(protocol: &str) -> String {
    match protocol.to_ascii_lowercase().as_str() {
        "h2" | "http/2" | "http/2.0" => "HTTP/2.0".into(),
        "h3" | "http/3" | "http/3.0" => "HTTP/3.0".into(),
        "http/1.0" => "HTTP/1.0".into(),
        _ => "HTTP/1.1".into(),
    }
}

/// A HAR entry from a BiDi `network.RequestData` and (when the request did
/// not fail) its `network.ResponseData`, with the bodies the page's data
/// collector kept.
pub fn har_entry(
    request: &Value,
    response: Option<&Value>,
    post_data: Option<&[u8]>,
    body: Option<&[u8]>,
    pageref: Option<&str>,
) -> Entry {
    let num = |v: &Value, k: &str| v.pointer(k).and_then(Value::as_f64).unwrap_or(-1.0);
    let t = &request["timings"];
    let span = |a: &str, b: &str| {
        let (a, b) = (num(t, a), num(t, b));
        if a > 0.0 && b > 0.0 { b - a } else { -1.0 }
    };
    let tls = num(t, "/tlsStart");
    let timings = Timings {
        dns: span("/dnsStart", "/dnsEnd"),
        connect: span("/connectStart", "/connectEnd"),
        ssl: if tls > 0.0 {
            span("/tlsStart", "/connectEnd")
        } else {
            -1.0
        },
        send: 0.0,
        wait: span("/requestStart", "/responseStart"),
        receive: span("/responseStart", "/responseEnd"),
    };
    let started = [num(t, "/requestTime"), num(t, "/timeOrigin")]
        .into_iter()
        .find(|v| *v > 0.0)
        .unwrap_or(0.0);
    let url = request["url"].as_str().unwrap_or_default();
    let headers = har_headers(&request["headers"]);
    let protocol = response
        .and_then(|r| r["protocol"].as_str())
        .unwrap_or("http/1.1");
    let har_request = HarRequest {
        method: request["method"].as_str().unwrap_or("GET").to_owned(),
        url: url.to_owned(),
        http_version: http_version(protocol),
        cookies: header(&headers, "cookie")
            .map(request_cookies)
            .unwrap_or_default(),
        query_string: query_string(url),
        post_data: post_data.map(|b| PostData {
            mime_type: header(&headers, "content-type")
                .unwrap_or_default()
                .to_owned(),
            text: String::from_utf8_lossy(b).into_owned(),
            params: Vec::new(),
        }),
        headers_size: request["headersSize"].as_i64().unwrap_or(-1),
        body_size: request["bodySize"].as_i64().unwrap_or(-1),
        headers,
    };
    let har_response = match response {
        None => HarResponse::default(),
        Some(r) => {
            let headers = har_headers(&r["headers"]);
            let mime = r["mimeType"]
                .as_str()
                .or_else(|| header(&headers, "content-type"))
                .unwrap_or("x-unknown");
            let mime = mime.split(';').next().unwrap_or(mime).trim().to_owned();
            let mut content = match body {
                Some(b) => Content::from_body(b, &mime),
                None => Content {
                    mime_type: mime,
                    ..Content::default()
                },
            };
            if let Some(size) = r.pointer("/content/size").and_then(Value::as_i64)
                && body.is_none()
            {
                content.size = size;
            }
            HarResponse {
                status: r["status"].as_i64().unwrap_or(-1),
                status_text: r["statusText"].as_str().unwrap_or_default().to_owned(),
                http_version: http_version(protocol),
                cookies: headers
                    .iter()
                    .filter(|h| h.name.eq_ignore_ascii_case("set-cookie"))
                    .flat_map(|h| {
                        h.value
                            .lines()
                            .filter_map(response_cookie)
                            .collect::<Vec<_>>()
                    })
                    .collect(),
                redirect_url: header(&headers, "location").unwrap_or_default().to_owned(),
                headers_size: r["headersSize"].as_i64().unwrap_or(-1),
                body_size: r["bodySize"].as_i64().unwrap_or(-1),
                content,
                headers,
            }
        }
    };
    Entry {
        pageref: pageref.map(str::to_owned),
        started_date_time: iso_time(started),
        time: timings.total(),
        request: har_request,
        response: har_response,
        timings,
        ..Entry::default()
    }
}

/// Where Playwright keeps its browsers, in lookup order, like the Chromium
/// discovery: `PLAYWRIGHT_BROWSERS_PATH` (unless empty or `"0"`), the
/// container's `/opt/pw-browsers` (not on Windows), the Linux and macOS
/// caches under the home directory, and `%LOCALAPPDATA%\ms-playwright`.
pub fn playwright_dirs(
    browsers_path: Option<PathBuf>,
    home: Option<PathBuf>,
    local_app_data: Option<PathBuf>,
    windows: bool,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(p) = browsers_path.filter(|p| !p.as_os_str().is_empty() && p.as_os_str() != "0") {
        dirs.push(p);
    }
    if !windows {
        dirs.push(PathBuf::from("/opt/pw-browsers"));
    }
    if let Some(home) = home {
        dirs.push(home.join(".cache/ms-playwright"));
        dirs.push(home.join("Library/Caches/ms-playwright"));
    }
    if let Some(local) = local_app_data {
        dirs.push(local.join("ms-playwright"));
    }
    dirs
}

#[cfg(test)]
mod tests {
    #[test]
    fn resource_types_from_bidi_destination_and_initiator() {
        use super::resource_type;
        assert_eq!(resource_type("document", None), "document");
        assert_eq!(resource_type("iframe", None), "document");
        assert_eq!(resource_type("style", Some("link")), "stylesheet");
        assert_eq!(resource_type("script", Some("script")), "script");
        assert_eq!(resource_type("image", Some("img")), "image");
        assert_eq!(resource_type("font", None), "font");
        assert_eq!(resource_type("video", None), "media");
        assert_eq!(resource_type("track", None), "texttrack");
        assert_eq!(resource_type("manifest", None), "manifest");
        assert_eq!(resource_type("", Some("fetch")), "fetch");
        assert_eq!(resource_type("", Some("xmlhttprequest")), "xhr");
        assert_eq!(resource_type("", Some("beacon")), "ping");
        assert_eq!(resource_type("", None), "other");
    }

    #[test]
    fn timing_from_bidi_is_relative_to_the_request_time() {
        use serde_json::json;
        let t = super::timing_from_bidi(&json!({
            "requestTime": 1000.0, "fetchStart": 1001.0, "dnsStart": 1002.0, "dnsEnd": 1003.0,
            "connectStart": 1004.0, "connectEnd": 1006.0, "tlsStart": 0, "tlsEnd": 1006.0,
            "requestStart": 1007.0, "responseStart": 1010.0, "responseEnd": 1012.0}));
        assert_eq!(t.start_time, 1000.0);
        assert_eq!(t.domain_lookup_start, 2.0);
        assert_eq!(t.domain_lookup_end, 3.0);
        assert_eq!(t.connect_start, 4.0);
        assert_eq!(t.secure_connection_start, -1.0);
        assert_eq!(t.connect_end, 6.0);
        assert_eq!(t.request_start, 7.0);
        assert_eq!(t.response_start, 10.0);
        assert_eq!(t.response_end, 12.0);
    }

    #[test]
    fn http_versions_from_bidi_protocols() {
        assert_eq!(super::http_version_of("http/1.1"), "HTTP/1.1");
        assert_eq!(super::http_version_of("h2"), "HTTP/2.0");
        assert_eq!(super::http_version_of("h3"), "HTTP/3.0");
        assert_eq!(super::http_version_of(""), "HTTP/1.1");
    }

    #[test]
    fn playwright_folders_on_every_system() {
        use std::path::PathBuf;
        let p = PathBuf::from;
        assert_eq!(
            playwright_dirs(Some(p("/pw")), Some(p("/home/u")), None, false),
            [
                p("/pw"),
                p("/opt/pw-browsers"),
                p("/home/u/.cache/ms-playwright"),
                p("/home/u/Library/Caches/ms-playwright"),
            ]
        );
        assert_eq!(
            playwright_dirs(
                None,
                Some(p("C:/Users/u")),
                Some(p("C:/Users/u/AppData/Local")),
                true
            ),
            [
                p("C:/Users/u/.cache/ms-playwright"),
                p("C:/Users/u/Library/Caches/ms-playwright"),
                p("C:/Users/u/AppData/Local/ms-playwright"),
            ]
        );
        // "0" and an empty value mean "the default folders" to Playwright.
        assert_eq!(
            playwright_dirs(Some(p("0")), None, None, false),
            [p("/opt/pw-browsers")]
        );
    }

    #[test]
    fn har_entries_from_bidi_network_events() {
        let request = serde_json::json!({
            "request": "7", "url": "http://h:1/p?a=1", "method": "POST",
            "headers": [{"name": "Content-Type", "value": {"type": "string", "value": "text/plain"}},
                        {"name": "Cookie", "value": {"type": "string", "value": "k=v"}}],
            "headersSize": 120, "bodySize": 3,
            "timings": {"timeOrigin": 1000.0, "requestTime": 1000.0, "dnsStart": 1.0, "dnsEnd": 2.0,
                        "connectStart": 2.0, "connectEnd": 4.0, "tlsStart": 0.0,
                        "requestStart": 5.0, "responseStart": 15.0, "responseEnd": 20.0}
        });
        let response = serde_json::json!({
            "url": "http://h:1/p?a=1", "protocol": "http/1.1", "status": 200, "statusText": "OK",
            "mimeType": "text/html;charset=utf-8",
            "headers": [{"name": "Content-Type", "value": {"type": "string", "value": "text/html; charset=utf-8"}},
                        {"name": "Set-Cookie", "value": {"type": "string", "value": "s=1; Path=/"}}],
            "headersSize": 80, "bodySize": 5, "content": {"size": 5}
        });
        let e = har_entry(
            &request,
            Some(&response),
            Some(b"x=1"),
            Some(b"hello"),
            Some("page@1"),
        );
        assert_eq!(e.pageref.as_deref(), Some("page@1"));
        assert_eq!(e.started_date_time, "1970-01-01T00:00:01.000Z");
        assert_eq!(e.request.method, "POST");
        assert_eq!(e.request.http_version, "HTTP/1.1");
        assert_eq!(e.request.query_string, [NameValuePair::new("a", "1")]);
        assert_eq!(e.request.cookies, [NameValuePair::new("k", "v")]);
        let post = e.request.post_data.as_ref().unwrap();
        assert_eq!(
            (post.mime_type.as_str(), post.text.as_str()),
            ("text/plain", "x=1")
        );
        assert_eq!(e.response.status, 200);
        assert_eq!(e.response.status_text, "OK");
        assert_eq!(e.response.cookies, [NameValuePair::new("s", "1")]);
        assert_eq!(e.response.content.mime_type, "text/html");
        assert_eq!(e.response.content.text.as_deref(), Some("hello"));
        assert_eq!(e.response.content.size, 5);
        assert_eq!(
            (
                e.timings.dns,
                e.timings.connect,
                e.timings.wait,
                e.timings.receive
            ),
            (1.0, 2.0, 10.0, 5.0)
        );
        assert_eq!(e.time, 18.0);

        // A request that failed has no response.
        let failed = har_entry(&request, None, None, None, None);
        assert_eq!(failed.response.status, -1);
        assert_eq!(failed.pageref, None);
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn playwright_arguments_with_the_bidi_port() {
        let args = command_line(Path::new("/tmp/p"), true, &["-foo".to_owned()]);
        assert_eq!(
            args,
            [
                "-no-remote",
                "-headless",
                "-profile",
                "/tmp/p",
                "--remote-debugging-port=0",
                "-foo",
                "about:blank"
            ]
        );
        let headed = command_line(Path::new("/tmp/p"), false, &[]);
        assert_eq!(&headed[1..3], ["-wait-for-browser", "-foreground"]);
    }

    #[test]
    fn headless_profiles_hide_scrollbars_like_juggler() {
        let files = profile_files(true);
        assert!(files.iter().any(|(p, c)| *p == "chrome/userContent.css"
            && c.contains("scrollbar-width: none !important")));
        assert!(files.iter().any(|(p, c)| *p == "user.js"
            && c.contains("toolkit.legacyUserProfileCustomizations.stylesheets")));
        assert!(profile_files(false).is_empty());
    }

    #[test]
    fn cookies_to_and_from_bidi() {
        let c = cookie_from_bidi(
            &json!({"name": "a", "value": {"type": "string", "value": "1"},
            "domain": "x.test", "path": "/", "httpOnly": true, "secure": false, "sameSite": "none",
            "size": 2}),
        );
        assert_eq!(
            (c.name.as_str(), c.value.as_str(), c.expires),
            ("a", "1", -1.0)
        );
        assert!(c.http_only);
        assert_eq!(c.same_site, SameSite::None);
        let resolved = NewCookie::new("a", "1")
            .url("http://x.test/p/q")
            .expires(2_000_000_000.0)
            .resolve()
            .unwrap();
        let v = cookie_to_bidi(&resolved);
        assert_eq!(v["value"], json!({"type": "string", "value": "1"}));
        assert_eq!(v["domain"], "x.test");
        assert_eq!(v["expiry"], 2_000_000_000u64);
    }

    #[test]
    fn init_scripts_run_as_scripts() {
        assert_eq!(
            init_script_function("var a = \"x\";"),
            r#"() => { (0, eval)("var a = \"x\";"); }"#
        );
    }

    #[test]
    fn webdriver_keys() {
        assert_eq!(webdriver_key("a").as_deref(), Some("a"));
        assert_eq!(webdriver_key("KeyA").as_deref(), Some("a"));
        assert_eq!(webdriver_key("Backspace").as_deref(), Some("\u{E003}"));
        assert_eq!(webdriver_key("Enter").as_deref(), Some("\u{E007}"));
        assert_eq!(webdriver_key("Control").as_deref(), Some("\u{E009}"));
        assert_eq!(webdriver_key("ArrowLeft").as_deref(), Some("\u{E012}"));
        assert_eq!(webdriver_key("F12").as_deref(), Some("\u{E03C}"));
        assert_eq!(webdriver_key("Space").as_deref(), Some(" "));
        assert_eq!(webdriver_key("NoSuchKey"), None);
    }

    #[test]
    fn unsupported_options_are_named() {
        assert!(unsupported_context_options(&ContextOptions::default()).is_empty());
        let o = ContextOptions {
            is_mobile: true,
            color_scheme: Some(ColorScheme::Dark),
            ..Default::default()
        };
        assert_eq!(unsupported_context_options(&o), ["isMobile", "colorScheme"]);
        let light = ContextOptions {
            color_scheme: Some(ColorScheme::Light),
            ..Default::default()
        };
        assert!(unsupported_context_options(&light).is_empty());
    }

    #[test]
    fn remote_values_become_json() {
        let v = json!({"type": "object", "value": [
            ["n", {"type": "number", "value": 1.5}],
            ["s", {"type": "string", "value": "x"}],
            ["u", {"type": "undefined"}],
            ["nan", {"type": "number", "value": "NaN"}],
            ["a", {"type": "array", "value": [{"type": "boolean", "value": true}, {"type": "null"}]}],
            ["d", {"type": "date", "value": "2026-11-23T00:00:00.000Z"}],
            ["el", {"type": "node", "sharedId": "x"}]
        ]});
        assert_eq!(
            remote_value_to_json(&v),
            json!({"n": 1.5, "s": "x", "u": null, "nan": null, "a": [true, null],
                   "d": "2026-11-23T00:00:00.000Z", "el": {}})
        );
        assert_eq!(
            remote_value_to_json(&json!({"type": "bigint", "value": "42"})),
            json!(42)
        );
    }
}
