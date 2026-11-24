//! Playwright's WebKit: command line, default user agent, context options
//! the engine cannot honour.

use std::path::Path;

/// The Safari version Playwright 1.65.1 reports for its WebKit build.
pub const BROWSER_VERSION: &str = "27.2";

/// Playwright's default WebKit user agent (a macOS Safari one on every
/// system, as in Playwright).
pub fn default_user_agent() -> String {
    format!(
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/{BROWSER_VERSION} Safari/605.1.15"
    )
}

/// Playwright's WebKit arguments: the inspector pipe, Windows' compositing
/// switch, `--headless`, then a persistent profile (and `about:blank`) or
/// no startup window, and the caller's arguments.
pub fn command_line(
    headless: bool,
    user_data_dir: Option<&Path>,
    args: &[String],
    windows: bool,
) -> Vec<String> {
    let mut out = vec!["--inspector-pipe".to_owned()];
    if windows {
        out.push("--disable-accelerated-compositing".to_owned());
    }
    if headless {
        out.push("--headless".to_owned());
    }
    match user_data_dir {
        Some(dir) => out.push(format!("--user-data-dir={}", dir.display())),
        None => out.push("--no-startup-window".to_owned()),
    }
    out.extend(args.iter().cloned());
    if user_data_dir.is_some() {
        out.push("about:blank".to_owned());
    }
    out
}

/// WebKit's `Input` modifier bits (Shift 1, Control 2, Alt 4, Meta 8) from
/// the CDP bits the shared keyboard model uses.
pub fn modifiers(cdp: u32) -> u32 {
    use crate::domain::input::{ALT, CONTROL, META, SHIFT};
    [(SHIFT, 1), (CONTROL, 2), (ALT, 4), (META, 8)]
        .iter()
        .filter(|(bit, _)| cdp & bit != 0)
        .map(|(_, wk)| wk)
        .sum()
}

/// A resolved cookie as `Playwright.setCookies` takes it (`WKBrowserContext.addCookies`):
/// `expires` in milliseconds, `session` when it has none.
pub fn cookie_to_webkit(c: &crate::domain::storage::NewCookie) -> serde_json::Value {
    use serde_json::json;
    let mut v = json!({"name": c.name, "value": c.value});
    if let Some(d) = &c.domain {
        v["domain"] = json!(d);
    }
    if let Some(p) = &c.path {
        v["path"] = json!(p);
    }
    if let Some(e) = c.expires
        && e != -1.0
    {
        v["expires"] = json!(e * 1000.0);
    }
    if let Some(h) = c.http_only {
        v["httpOnly"] = json!(h);
    }
    if let Some(s) = c.secure {
        v["secure"] = json!(s);
    }
    if let Some(s) = c.same_site {
        v["sameSite"] = json!(s.as_str());
    }
    if let Some(k) = &c.partition_key {
        v["partitionKey"] = json!(k);
    }
    v["session"] = json!(matches!(c.expires, None | Some(-1.0)));
    v
}

/// A `Playwright.getAllCookies` cookie (`expires` in milliseconds, `-1` for session).
pub fn cookie_from_webkit(c: &serde_json::Value) -> crate::domain::storage::Cookie {
    use serde_json::Value;
    let mut cookie = crate::domain::storage::Cookie::from_protocol(c);
    let expires = c.get("expires").and_then(Value::as_f64).unwrap_or(-1.0);
    cookie.expires = if expires == -1.0 || c["session"] == true {
        -1.0
    } else {
        expires / 1000.0
    };
    cookie.partition_key = c
        .get("partitionKey")
        .and_then(Value::as_str)
        .map(str::to_owned);
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playwright_arguments() {
        assert_eq!(
            command_line(true, None, &["--foo".into()], false),
            [
                "--inspector-pipe",
                "--headless",
                "--no-startup-window",
                "--foo"
            ]
        );
        assert_eq!(
            command_line(false, Some(Path::new("/p")), &[], true),
            [
                "--inspector-pipe",
                "--disable-accelerated-compositing",
                "--user-data-dir=/p",
                "about:blank"
            ]
        );
    }

    #[test]
    fn modifiers_use_webkits_bits() {
        use crate::domain::input::{ALT, CONTROL, META, SHIFT};
        assert_eq!(modifiers(SHIFT), 1);
        assert_eq!(modifiers(CONTROL), 2);
        assert_eq!(modifiers(ALT), 4);
        assert_eq!(modifiers(META), 8);
        assert_eq!(modifiers(SHIFT | META), 9);
        assert_eq!(modifiers(0), 0);
    }

    #[test]
    fn default_user_agent_is_playwrights() {
        assert_eq!(
            default_user_agent(),
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Safari/605.1.15"
        );
    }

    #[test]
    fn cookies_to_webkit_like_playwright() {
        use crate::domain::storage::{NewCookie, SameSite};
        let c = NewCookie::new("a", "b")
            .url("http://127.0.0.1:8000/x/y.html")
            .resolve()
            .unwrap();
        assert_eq!(
            cookie_to_webkit(&c),
            serde_json::json!({"name": "a", "value": "b", "domain": "127.0.0.1",
                "path": "/x/", "secure": false, "session": true})
        );
        let c = NewCookie::new("a", "b")
            .domain("x.test")
            .path("/")
            .expires(1_700_000_000.0)
            .http_only(true)
            .same_site(SameSite::Strict);
        assert_eq!(
            cookie_to_webkit(&c),
            serde_json::json!({"name": "a", "value": "b", "domain": "x.test", "path": "/",
                "expires": 1_700_000_000_000.0, "httpOnly": true, "sameSite": "Strict",
                "session": false})
        );
    }

    #[test]
    fn cookies_from_webkit_like_playwright() {
        use crate::domain::storage::SameSite;
        let c = cookie_from_webkit(&serde_json::json!({"name": "a", "value": "b",
            "domain": "x.test", "path": "/", "expires": 1_700_000_000_000.0_f64,
            "httpOnly": true, "secure": true, "sameSite": "None", "session": false}));
        assert_eq!((c.name.as_str(), c.value.as_str()), ("a", "b"));
        assert_eq!(c.expires, 1_700_000_000.0);
        assert!(c.http_only && c.secure);
        assert_eq!(c.same_site, SameSite::None);
        let c = cookie_from_webkit(&serde_json::json!({"name": "s", "value": "", "domain": "d",
            "path": "/", "expires": -1, "httpOnly": false, "secure": false,
            "sameSite": "Lax", "session": true}));
        assert_eq!(c.expires, -1.0);
    }
}
