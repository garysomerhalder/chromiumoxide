//! Requests and responses on the WebKit engine (`PageEvent::Request` and
//! friends, and the request a route intercepted).

/// A network request (from a route, or `PageEvent::Request` and friends).
#[derive(Debug, Clone)]
pub struct Request {
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) failure: Option<String>,
}

/// A response's status line and headers (`PageEvent::Response`).
#[derive(Debug, Clone)]
pub struct Response {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl Response {
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn status(&self) -> u16 {
        self.status
    }
    pub fn status_text(&self) -> &str {
        &self.status_text
    }
    /// `response.ok()`: status 200..=299 (or 0 for non-HTTP URLs).
    pub fn ok(&self) -> bool {
        self.status == 0 || (200..300).contains(&self.status)
    }
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
    /// The first header with this name (case-insensitive).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

impl Request {
    /// `request.failure()`: the error text of a failed request.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn method(&self) -> &str {
        &self.method
    }
    /// Request headers, in order.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }
    /// The first header with this name (case-insensitive).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}
