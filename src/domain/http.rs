//! The HTTP client behind `APIRequestContext` (Playwright's `fetch.ts`):
//! request bodies, redirects and cookies, without I/O.

use crate::domain::storage::{NewCookie, SameSite};

/// A request for the HTTP client port.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout_ms: Option<u64>,
    pub ignore_https_errors: bool,
    /// The client certificate to present, if the server asks.
    pub client_identity: Option<crate::domain::client_cert::ClientIdentity>,
}

/// A response from the HTTP client port (no redirects followed).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HttpResponse {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// The server's IP address and port.
    pub server_addr: Option<(String, u16)>,
    /// TLS protocol (e.g. `TLSv1.3`) and the server's certificate (DER).
    pub tls: Option<(String, Option<Vec<u8>>)>,
    pub timing: Option<crate::domain::network::Timing>,
}

/// A `multipart` field.
#[derive(Debug, Clone, PartialEq)]
pub enum MultipartValue {
    Text(String),
    File {
        name: String,
        mime_type: String,
        buffer: Vec<u8>,
    },
}

/// `application/x-www-form-urlencoded` (spaces as `+`).
pub fn form_urlencode(pairs: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

/// Adds `params` to the URL's query.
pub fn append_params(url: &str, params: &[(String, String)]) -> String {
    if params.is_empty() {
        return url.to_owned();
    }
    match url::Url::parse(url) {
        Ok(mut u) => {
            u.query_pairs_mut().extend_pairs(params);
            u.to_string()
        }
        Err(_) => {
            let sep = if url.contains('?') { '&' } else { '?' };
            format!("{url}{sep}{}", form_urlencode(params))
        }
    }
}

/// A `multipart/form-data` body with Playwright's layout.
pub fn multipart_body(boundary: &str, fields: &[(String, MultipartValue)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, value) in fields {
        out.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match value {
            MultipartValue::Text(v) => {
                out.extend_from_slice(
                    format!("content-disposition: form-data; name=\"{name}\"\r\n\r\n{v}\r\n")
                        .as_bytes(),
                );
            }
            MultipartValue::File {
                name: file,
                mime_type,
                buffer,
            } => {
                out.extend_from_slice(
                    format!(
                        "content-disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\ncontent-type: {mime_type}\r\n\r\n"
                    )
                    .as_bytes(),
                );
                out.extend_from_slice(buffer);
                out.extend_from_slice(b"\r\n");
            }
        }
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    out
}

/// Playwright's boundary: `----WebKitFormBoundary` + 16 characters from `random`.
pub fn multipart_boundary(random: &[u8; 16]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789AB";
    let tail: String = random
        .iter()
        .map(|b| ALPHABET[(*b as usize) % 64] as char)
        .collect();
    format!("----WebKitFormBoundary{tail}")
}

/// The next request of a redirect (Playwright's rules), or `None` when the
/// response is not a redirect.
pub fn redirect_request(req: &HttpRequest, status: u16, location: &str) -> Option<HttpRequest> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) || location.is_empty() {
        return None;
    }
    let from = url::Url::parse(&req.url).ok()?;
    let to = from.join(location).ok()?;
    let mut next = req.clone();
    next.url = to.to_string();
    next.headers
        .retain(|(k, _)| !k.eq_ignore_ascii_case("cookie"));
    let to_get = (matches!(status, 301 | 302) && req.method == "POST")
        || (status == 303 && !matches!(req.method.as_str(), "GET" | "HEAD"));
    if to_get {
        next.method = "GET".into();
        next.body.clear();
        next.headers.retain(|(k, _)| {
            !matches!(
                k.to_ascii_lowercase().as_str(),
                "content-encoding"
                    | "content-language"
                    | "content-length"
                    | "content-location"
                    | "content-type"
            )
        });
    }
    if from.origin() != to.origin() {
        next.headers
            .retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
    }
    Some(next)
}

/// The `Cookie` request header for `cookies` (Playwright's
/// `serializeCookieNameValue`: a nameless cookie is just its value).
pub fn cookie_header(cookies: &[crate::domain::storage::Cookie]) -> String {
    cookies
        .iter()
        .map(|c| {
            if c.name.is_empty() {
                c.value.clone()
            } else {
                format!("{}={}", c.name, c.value)
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A `Set-Cookie` header as a cookie for `request_url` (`now_s`: Unix seconds).
pub fn parse_set_cookie(header: &str, request_url: &str, now_s: f64) -> Option<NewCookie> {
    let url = url::Url::parse(request_url).ok()?;
    let mut parts = header.split(';');
    // Playwright 1.65 `parseCookieNameValue`: no `=` means a nameless cookie.
    let pair = parts.next()?;
    let (name, value) = pair.split_once('=').unwrap_or(("", pair));
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() && value.is_empty() {
        return None;
    }
    let mut c = NewCookie::new(name, value);
    let mut expires = -1.0;
    let mut max_age = None;
    let mut domain = None;
    let mut path = None;
    let mut same_site = SameSite::Lax;
    let (mut secure, mut http_only) = (false, false);
    for attr in parts {
        let (k, v) = attr
            .split_once('=')
            .map_or((attr.trim(), ""), |(k, v)| (k.trim(), v.trim()));
        match k.to_ascii_lowercase().as_str() {
            "expires" => {
                if let Some(t) = http_date(v) {
                    expires = t;
                }
            }
            "max-age" => {
                let digits = v.strip_prefix('-').unwrap_or(v);
                if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                    max_age = v.parse::<f64>().ok();
                }
            }
            "domain" if !v.is_empty() => {
                let d = v.to_ascii_lowercase();
                domain = Some(if d.starts_with('.') || !d.contains('.') {
                    d
                } else {
                    format!(".{d}")
                });
            }
            "path" if v.starts_with('/') => path = Some(v.to_owned()),
            "secure" => secure = true,
            "httponly" => http_only = true,
            "samesite" => {
                same_site = match v.to_ascii_lowercase().as_str() {
                    "strict" => SameSite::Strict,
                    "none" => SameSite::None,
                    _ => SameSite::Lax,
                }
            }
            _ => {}
        }
    }
    if let Some(age) = max_age {
        expires = if age <= 0.0 { 0.0 } else { now_s + age };
    }
    let default_path = || {
        let p = url.path();
        match p.rfind('/') {
            Some(0) | None => "/".to_owned(),
            Some(i) => p[..i].to_owned(),
        }
    };
    let host = url.host_str().unwrap_or_default().to_owned();
    let domain = domain.unwrap_or_else(|| host.clone());
    if !crate::domain::storage::domain_matches(&host, &domain) {
        return None;
    }
    c.domain = Some(domain);
    c.path = Some(path.unwrap_or_else(default_path));
    c.expires = Some(expires);
    c.secure = Some(secure);
    c.http_only = Some(http_only);
    c.same_site = Some(same_site);
    Some(c)
}

/// An IMF-fixdate (`Wed, 21 Oct 2015 07:28:00 GMT`) in Unix seconds.
fn http_date(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_, day, month, year, time, ..] = parts.as_slice() else {
        return None;
    };
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let m = months
        .iter()
        .position(|x| month.to_ascii_lowercase().starts_with(x))? as i64
        + 1;
    let d: i64 = day.trim_end_matches('-').parse().ok()?;
    let y: i64 = year.parse().ok()?;
    let t: Vec<i64> = time.split(':').filter_map(|p| p.parse().ok()).collect();
    let [h, mi, sec] = t.as_slice() else {
        return None;
    };
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec) as f64)
}

/// The `failOnStatusCode` error text.
pub fn status_error(status: u16, status_text: &str, body: &[u8]) -> String {
    if body.is_empty() {
        return format!("{status} {status_text}");
    }
    let mut text = String::from_utf8_lossy(body).into_owned();
    if text.chars().count() > 1000 {
        text = text.chars().take(997).collect::<String>() + "...";
    }
    format!("{status} {status_text}\nResponse text:\n{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect()
    }

    #[test]
    fn params_and_forms_like_playwright() {
        assert_eq!(
            append_params("http://h/dump?x=1", &pairs(&[("y", "2"), ("z", "a b")])),
            "http://h/dump?x=1&y=2&z=a+b"
        );
        assert_eq!(
            append_params("http://h/p", &pairs(&[("a", "1")])),
            "http://h/p?a=1"
        );
        assert_eq!(append_params("http://h/p", &[]), "http://h/p");
        assert_eq!(
            form_urlencode(&pairs(&[("a", "1"), ("b", "x y")])),
            "a=1&b=x+y"
        );
    }

    #[test]
    fn multipart_like_playwright() {
        let b = "----WebKitFormBoundaryWgAlzGB3E00ReAkp";
        let body = multipart_body(
            b,
            &[
                ("f".into(), MultipartValue::Text("v".into())),
                (
                    "file".into(),
                    MultipartValue::File {
                        name: "a.txt".into(),
                        mime_type: "text/plain".into(),
                        buffer: b"hi".to_vec(),
                    },
                ),
            ],
        );
        assert_eq!(
            String::from_utf8(body).unwrap(),
            "------WebKitFormBoundaryWgAlzGB3E00ReAkp\r\ncontent-disposition: form-data; name=\"f\"\r\n\r\nv\r\n------WebKitFormBoundaryWgAlzGB3E00ReAkp\r\ncontent-disposition: form-data; name=\"file\"; filename=\"a.txt\"\r\ncontent-type: text/plain\r\n\r\nhi\r\n------WebKitFormBoundaryWgAlzGB3E00ReAkp--\r\n"
        );
        let boundary = multipart_boundary(&[0; 16]);
        assert!(boundary.starts_with("----WebKitFormBoundary"));
        assert_eq!(boundary.len(), 38);
    }

    #[test]
    fn redirects_like_playwright() {
        let post = HttpRequest {
            method: "POST".into(),
            url: "http://h/a/b".into(),
            headers: vec![
                ("content-type".into(), "text/plain".into()),
                ("cookie".into(), "k=v".into()),
                ("authorization".into(), "Basic x".into()),
                ("x".into(), "1".into()),
            ],
            body: b"x".to_vec(),
            ..Default::default()
        };
        let next = redirect_request(&post, 302, "/dump").unwrap();
        assert_eq!(next.method, "GET");
        assert_eq!(next.url, "http://h/dump");
        assert!(next.body.is_empty());
        assert_eq!(
            next.headers,
            vec![
                ("authorization".into(), "Basic x".into()),
                ("x".into(), "1".into())
            ]
        );
        // 307 keeps the method and body; another origin drops authorization.
        let next = redirect_request(&post, 307, "http://other/c").unwrap();
        assert_eq!(
            (next.method.as_str(), next.body.as_slice()),
            ("POST", &b"x"[..])
        );
        assert!(!next.headers.iter().any(|(k, _)| k == "authorization"));
        assert!(!next.headers.iter().any(|(k, _)| k == "cookie"));
        // 303 turns PUT into GET; 301 keeps PUT.
        let put = HttpRequest {
            method: "PUT".into(),
            ..post.clone()
        };
        assert_eq!(redirect_request(&put, 303, "/x").unwrap().method, "GET");
        assert_eq!(redirect_request(&put, 301, "/x").unwrap().method, "PUT");
        assert!(redirect_request(&post, 200, "/x").is_none());
        assert!(redirect_request(&post, 302, "").is_none());
    }

    #[test]
    fn set_cookie_headers() {
        let c =
            parse_set_cookie("from_server=yes; Path=/", "http://127.0.0.1:5/dump", 1000.0).unwrap();
        assert_eq!((c.name.as_str(), c.value.as_str()), ("from_server", "yes"));
        assert_eq!(c.domain.as_deref(), Some("127.0.0.1"));
        assert_eq!(c.path.as_deref(), Some("/"));
        assert_eq!(c.expires, Some(-1.0));
        assert_eq!(c.same_site, Some(SameSite::Lax));
        let c = parse_set_cookie(
            "s=1; Domain=.example.com; Max-Age=60; Secure; HttpOnly; SameSite=None",
            "https://a.example.com/x/y",
            1000.0,
        )
        .unwrap();
        assert_eq!(c.domain.as_deref(), Some(".example.com"));
        assert_eq!(
            c.path.as_deref(),
            Some("/x"),
            "default path is the directory"
        );
        assert_eq!(c.expires, Some(1060.0));
        assert_eq!(
            (c.secure, c.http_only, c.same_site),
            (Some(true), Some(true), Some(SameSite::None))
        );
        let c = parse_set_cookie(
            "e=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            "http://h/",
            0.0,
        )
        .unwrap();
        assert_eq!(c.expires, Some(1_445_412_480.0));
        // Playwright 1.65 (cookieStore.ts): a pair without `=` is a nameless cookie.
        let c = parse_set_cookie("token", "http://h/", 0.0).unwrap();
        assert_eq!((c.name.as_str(), c.value.as_str()), ("", "token"));
        assert!(parse_set_cookie(" ; Path=/", "http://h/", 0.0).is_none());
        // A dotless Domain stays host-only; Max-Age must be an integer.
        let c = parse_set_cookie(
            "a=1; Domain=localhost; Max-Age=1.5",
            "http://localhost/",
            0.0,
        )
        .unwrap();
        assert_eq!(
            (c.domain.as_deref(), c.expires),
            (Some("localhost"), Some(-1.0))
        );
        // A Domain the response host does not belong to drops the cookie.
        assert!(parse_set_cookie("a=1; Domain=other.test", "http://a.example.com/", 0.0).is_none());
    }

    #[test]
    fn cookie_headers_serialize_nameless_cookies_as_their_value() {
        use crate::domain::storage::{Cookie, SameSite};
        let c = |name: &str, value: &str| Cookie {
            name: name.into(),
            value: value.into(),
            domain: "h".into(),
            path: "/".into(),
            expires: -1.0,
            http_only: false,
            secure: false,
            same_site: SameSite::Lax,
            partition_key: None,
        };
        assert_eq!(cookie_header(&[c("a", "1"), c("", "x")]), "a=1; x");
    }

    #[test]
    fn status_errors() {
        assert_eq!(
            status_error(404, "Not Found", b"nope"),
            "404 Not Found\nResponse text:\nnope"
        );
        assert_eq!(
            status_error(500, "Internal Server Error", b""),
            "500 Internal Server Error"
        );
        let long = "x".repeat(1200);
        assert_eq!(
            status_error(400, "Bad Request", long.as_bytes()).len(),
            "400 Bad Request\nResponse text:\n".len() + 1000
        );
    }
}
