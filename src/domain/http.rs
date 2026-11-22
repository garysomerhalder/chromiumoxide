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
}

/// A response from the HTTP client port (no redirects followed).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HttpResponse {
    pub url: String,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
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
    unimplemented!()
}

/// Adds `params` to the URL's query.
pub fn append_params(url: &str, params: &[(String, String)]) -> String {
    unimplemented!()
}

/// A `multipart/form-data` body with Playwright's layout.
pub fn multipart_body(boundary: &str, fields: &[(String, MultipartValue)]) -> Vec<u8> {
    unimplemented!()
}

/// Playwright's boundary: `----WebKitFormBoundary` + 16 characters from `random`.
pub fn multipart_boundary(random: &[u8; 16]) -> String {
    unimplemented!()
}

/// The next request of a redirect (Playwright's rules), or `None` when the
/// response is not a redirect.
pub fn redirect_request(req: &HttpRequest, status: u16, location: &str) -> Option<HttpRequest> {
    unimplemented!()
}

/// A `Set-Cookie` header as a cookie for `request_url` (`now_s`: Unix seconds).
pub fn parse_set_cookie(header: &str, request_url: &str, now_s: f64) -> Option<NewCookie> {
    unimplemented!()
}

/// The `failOnStatusCode` error text.
pub fn status_error(status: u16, status_text: &str, body: &[u8]) -> String {
    unimplemented!()
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
        assert!(parse_set_cookie("novalue", "http://h/", 0.0).is_none());
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
