//! Cookies, storage state and permissions, as Playwright models them.

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
