//! Firefox: launch arguments and WebDriver BiDi value conversion (pure).

use std::path::Path;

use serde_json::{Map, Number, Value};

use crate::domain::emulation::{ColorScheme, ContextOptions, ServiceWorkers};
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
    check(!o.permissions.is_empty(), "permissions");
    check(o.offline, "offline");
    check(o.proxy.is_some(), "proxy");
    check(!o.client_certificates.is_empty(), "clientCertificates");
    check(!o.java_script_enabled, "javaScriptEnabled");
    check(o.bypass_csp, "bypassCSP");
    check(o.base_url.is_some(), "baseURL");
    check(o.service_workers != ServiceWorkers::Allow, "serviceWorkers");
    check(o.record_har.is_some(), "recordHar");
    check(o.record_video.is_some(), "recordVideo");
    out
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

#[cfg(test)]
mod tests {
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
