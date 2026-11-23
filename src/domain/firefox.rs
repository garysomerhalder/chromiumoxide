//! Firefox: launch arguments and WebDriver BiDi value conversion (pure).

use std::path::Path;

use serde_json::{Map, Number, Value};

use crate::domain::emulation::{ColorScheme, ContextOptions, ServiceWorkers};

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
    check(o.http_credentials.is_some(), "httpCredentials");
    check(o.proxy.is_some(), "proxy");
    check(!o.client_certificates.is_empty(), "clientCertificates");
    check(!o.java_script_enabled, "javaScriptEnabled");
    check(o.bypass_csp, "bypassCSP");
    check(o.base_url.is_some(), "baseURL");
    check(o.storage_state.is_some(), "storageState");
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
