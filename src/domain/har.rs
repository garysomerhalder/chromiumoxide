//! HAR 1.2 (HTTP Archive) as Playwright records and replays it.

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
