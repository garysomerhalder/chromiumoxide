//! Network model: header lists, resource types and request timing, as
//! Playwright exposes them.

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
