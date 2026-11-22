//! Network model: header lists, resource types and request timing, as
//! Playwright exposes them.

use std::collections::HashMap;

use serde_json::Value;

/// One header line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderEntry {
    pub name: String,
    pub value: String,
}

impl HeaderEntry {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        HeaderEntry {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// An ordered header list with case-insensitive lookup (Playwright's `RawHeaders`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawHeaders {
    entries: Vec<HeaderEntry>,
}

impl RawHeaders {
    pub fn new(entries: Vec<HeaderEntry>) -> Self {
        RawHeaders { entries }
    }

    /// From a protocol `Headers` object. With `split`, values holding several
    /// lines (raw headers, e.g. repeated `Set-Cookie`) become several entries.
    pub fn from_protocol(headers: &Value, split: bool) -> Self {
        let mut entries = Vec::new();
        if let Some(obj) = headers.as_object() {
            for (name, value) in obj {
                let value = match value {
                    Value::String(s) => s.clone(),
                    Value::Null => continue,
                    other => other.to_string(),
                };
                if split {
                    for v in value.split('\n') {
                        entries.push(HeaderEntry::new(name, v.trim()));
                    }
                } else {
                    entries.push(HeaderEntry::new(name, value));
                }
            }
        }
        RawHeaders { entries }
    }

    pub fn entries(&self) -> &[HeaderEntry] {
        &self.entries
    }

    /// Every value of `name`, in order.
    pub fn get_all(&self, name: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.name.eq_ignore_ascii_case(name))
            .map(|e| e.value.clone())
            .collect()
    }

    /// Values of `name` joined with `, ` (`\n` for `set-cookie`).
    pub fn get(&self, name: &str) -> Option<String> {
        let values = self.get_all(name);
        if values.is_empty() {
            return None;
        }
        let sep = if name.eq_ignore_ascii_case("set-cookie") {
            "\n"
        } else {
            ", "
        };
        Some(values.join(sep))
    }

    /// Lower-cased names to joined values.
    pub fn to_map(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for e in &self.entries {
            let name = e.name.to_lowercase();
            if !map.contains_key(&name)
                && let Some(v) = self.get(&name)
            {
                map.insert(name, v);
            }
        }
        map
    }
}

/// Playwright's resource type: the protocol type, lower-cased.
pub fn resource_type(protocol_type: Option<&str>) -> String {
    protocol_type.unwrap_or("Other").to_lowercase()
}

/// Request timing in milliseconds (Playwright's `request.timing()`).
/// `start_time` is wall-clock; the others are relative to it, `-1` if unknown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    pub start_time: f64,
    pub domain_lookup_start: f64,
    pub domain_lookup_end: f64,
    pub connect_start: f64,
    pub secure_connection_start: f64,
    pub connect_end: f64,
    pub request_start: f64,
    pub response_start: f64,
    pub response_end: f64,
}

impl Timing {
    /// From a response's protocol `timing`, the request's `wallTime` (s) and
    /// monotonic `timestamp` (s).
    pub fn from_protocol(timing: Option<&Value>, wall_time: f64, timestamp: f64) -> Self {
        let f = |t: &Value, k: &str| t.get(k).and_then(Value::as_f64).unwrap_or(-1.0);
        match timing {
            Some(t) if t.get("requestTime").is_some() => Timing {
                start_time: (f(t, "requestTime") - timestamp + wall_time) * 1000.0,
                domain_lookup_start: f(t, "dnsStart"),
                domain_lookup_end: f(t, "dnsEnd"),
                connect_start: f(t, "connectStart"),
                secure_connection_start: f(t, "sslStart"),
                connect_end: f(t, "connectEnd"),
                request_start: f(t, "sendStart"),
                response_start: f(t, "receiveHeadersEnd"),
                response_end: -1.0,
            },
            _ => Timing::unknown(wall_time),
        }
    }

    /// Only the start time is known.
    pub fn unknown(wall_time: f64) -> Self {
        Timing {
            start_time: wall_time * 1000.0,
            domain_lookup_start: -1.0,
            domain_lookup_end: -1.0,
            connect_start: -1.0,
            secure_connection_start: -1.0,
            connect_end: -1.0,
            request_start: -1.0,
            response_start: -1.0,
            response_end: -1.0,
        }
    }

    /// Records the end of the response at monotonic `finished_at` (s).
    pub fn finish(&mut self, request_timestamp: f64, finished_at: f64) {
        let ms = ((finished_at - request_timestamp) * 1000.0 * 1000.0).round() / 1000.0;
        self.response_end = ms.max(self.response_start);
    }
}

/// The standard reason phrase for a status code (empty when unknown).
pub fn status_text(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn headers_from_a_protocol_object_split_raw_values() {
        let obj = json!({"Content-Type": "text/html", "Set-Cookie": "a=1\nb=2"});
        let plain = RawHeaders::from_protocol(&obj, false);
        assert_eq!(plain.entries().len(), 2);
        let raw = RawHeaders::from_protocol(&obj, true);
        assert_eq!(raw.get_all("set-cookie"), ["a=1", "b=2"]);
        assert_eq!(raw.get("SET-COOKIE").as_deref(), Some("a=1\nb=2"));
        assert_eq!(raw.get("content-type").as_deref(), Some("text/html"));
        assert_eq!(raw.get("missing"), None);
    }

    #[test]
    fn header_map_lowercases_and_joins_duplicates() {
        let h = RawHeaders::new(vec![
            HeaderEntry::new("Accept", "a"),
            HeaderEntry::new("accept", "b"),
            HeaderEntry::new("Set-Cookie", "x=1"),
            HeaderEntry::new("set-cookie", "y=2"),
        ]);
        let map = h.to_map();
        assert_eq!(map["accept"], "a, b");
        assert_eq!(map["set-cookie"], "x=1\ny=2");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn resource_types_are_lowercased() {
        assert_eq!(resource_type(Some("XHR")), "xhr");
        assert_eq!(resource_type(Some("Document")), "document");
        assert_eq!(resource_type(None), "other");
    }

    #[test]
    fn timing_follows_playwright() {
        let t = Timing::from_protocol(
            Some(&json!({
                "requestTime": 100.5, "dnsStart": 1.0, "dnsEnd": 2.0, "connectStart": 3.0,
                "sslStart": -1.0, "connectEnd": 4.0, "sendStart": 5.0, "receiveHeadersEnd": 6.0
            })),
            1_700_000_000.0,
            100.0,
        );
        assert_eq!(t.start_time, (100.5 - 100.0 + 1_700_000_000.0) * 1000.0);
        assert_eq!(
            (t.domain_lookup_start, t.domain_lookup_end, t.connect_start),
            (1.0, 2.0, 3.0)
        );
        assert_eq!(
            (
                t.secure_connection_start,
                t.connect_end,
                t.request_start,
                t.response_start
            ),
            (-1.0, 4.0, 5.0, 6.0)
        );
        assert_eq!(t.response_end, -1.0);
        let none = Timing::from_protocol(None, 10.0, 1.0);
        assert_eq!((none.start_time, none.response_start), (10_000.0, -1.0));
        // responseEnd is relative to the request's timestamp, at least responseStart.
        let mut t = t;
        t.finish(100.0, 100.002);
        assert_eq!(t.response_end, 6.0);
        t.finish(100.0, 100.5);
        assert_eq!(t.response_end, 500.0);
    }

    #[test]
    fn status_text_defaults() {
        assert_eq!(status_text(201), "Created");
        assert_eq!(status_text(404), "Not Found");
        assert_eq!(status_text(799), "");
    }
}
