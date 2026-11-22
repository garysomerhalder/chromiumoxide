//! The CDP commands Playwright sends to initialise a page's main session
//! (`FrameSession._initialize` in `crPage.ts`), as a pure plan.

use serde_json::{Value, json};

use crate::domain::emulation::{ContextOptions, FontPlatform, Viewport};

/// Inputs that are only known at runtime.
#[derive(Debug, Clone)]
pub struct InitContext<'a> {
    pub options: &'a ContextOptions,
    pub headless: bool,
    pub font_platform: FontPlatform,
    /// From `Browser.getWindowForTarget`.
    pub window_id: Option<i64>,
    pub utility_world: &'a str,
}

/// The ordered `(method, params)` list, excluding `Page.getFrameTree` and the
/// final `Runtime.runIfWaitingForDebugger`.
pub fn page_init_commands(_c: &InitContext<'_>) -> Vec<(&'static str, Value)> {
    unimplemented!()
}

/// Commands that apply a viewport (used at init and by `setViewportSize`).
pub fn viewport_commands(
    _options: &ContextOptions,
    _window_id: Option<i64>,
    _resized: Option<Viewport>,
) -> Vec<(&'static str, Value)> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::emulation::{ColorScheme, HttpCredentials, ReducedMotion};

    const WORLD: &str = "__chromiumoxide_utility_world__";

    fn probe_options() -> ContextOptions {
        // The options of the recorded Playwright 1.63 session (see docs).
        ContextOptions {
            viewport: Some(Viewport::new(400, 300)),
            device_scale_factor: Some(2.0),
            is_mobile: true,
            has_touch: true,
            user_agent: Some("UA-X".into()),
            locale: Some("de-DE".into()),
            timezone_id: Some("Europe/Berlin".into()),
            color_scheme: Some(ColorScheme::Dark),
            reduced_motion: Some(ReducedMotion::Reduce),
            http_credentials: Some(HttpCredentials {
                username: "u".into(),
                password: "p".into(),
                origin: None,
            }),
            ..ContextOptions::default()
        }
    }

    #[test]
    fn matches_recorded_playwright_traffic() {
        let o = probe_options();
        let got = page_init_commands(&InitContext {
            options: &o,
            headless: true,
            font_platform: FontPlatform::Linux,
            window_id: Some(1),
            utility_world: WORLD,
        });
        let want: Vec<(&str, Value)> = vec![
            ("Page.enable", json!({})),
            ("Log.enable", json!({})),
            ("Page.setLifecycleEventsEnabled", json!({"enabled": true})),
            ("Runtime.enable", json!({})),
            (
                "Page.addScriptToEvaluateOnNewDocument",
                json!({"source": "", "worldName": WORLD}),
            ),
            ("Network.enable", json!({})),
            ("Network.setCacheDisabled", json!({"cacheDisabled": true})),
            (
                "Fetch.enable",
                json!({"handleAuthRequests": true, "patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
            ),
            (
                "Target.setAutoAttach",
                json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
            ),
            (
                "Emulation.setFocusEmulationEnabled",
                json!({"enabled": true}),
            ),
            (
                "Browser.setWindowBounds",
                json!({"windowId": 1, "bounds": {"width": 400, "height": 300}}),
            ),
            (
                "Emulation.setDeviceMetricsOverride",
                json!({"mobile": true, "width": 400, "height": 300, "screenWidth": 400, "screenHeight": 300, "deviceScaleFactor": 2.0, "screenOrientation": {"angle": 90, "type": "landscapePrimary"}}),
            ),
            (
                "Emulation.setTouchEmulationEnabled",
                json!({"enabled": true}),
            ),
            (
                "Emulation.setUserAgentOverride",
                json!({"userAgent": "UA-X", "acceptLanguage": "de-DE", "userAgentMetadata": {"mobile": true, "model": "", "architecture": "x86", "platform": "Windows", "platformVersion": ""}}),
            ),
            ("Emulation.setLocaleOverride", json!({"locale": "de-DE"})),
            (
                "Emulation.setTimezoneOverride",
                json!({"timezoneId": "Europe/Berlin"}),
            ),
            (
                "Page.setFontFamilies",
                json!({"fontFamilies": {"standard": "Times New Roman", "fixed": "Monospace", "serif": "Times New Roman", "sansSerif": "Arial", "cursive": "Comic Sans MS", "fantasy": "Impact"}}),
            ),
            (
                "Emulation.setEmulatedMedia",
                json!({"media": "", "features": [{"name": "prefers-color-scheme", "value": "dark"}, {"name": "prefers-reduced-motion", "value": "reduce"}, {"name": "forced-colors", "value": "none"}, {"name": "prefers-contrast", "value": "no-preference"}]}),
            ),
        ];
        assert_eq!(got.len(), want.len(), "{got:#?}");
        for (g, w) in got.iter().zip(want.iter()) {
            assert_eq!(g, w);
        }
    }

    #[test]
    fn defaults_have_no_interception_and_desktop_metrics() {
        let o = ContextOptions::default();
        let got = page_init_commands(&InitContext {
            options: &o,
            headless: true,
            font_platform: FontPlatform::Linux,
            window_id: None,
            utility_world: WORLD,
        });
        let methods: Vec<_> = got.iter().map(|(m, _)| *m).collect();
        assert!(!methods.contains(&"Fetch.enable"));
        assert!(
            !methods.contains(&"Browser.setWindowBounds"),
            "no window id"
        );
        assert!(!methods.contains(&"Emulation.setTouchEmulationEnabled"));
        assert!(!methods.contains(&"Emulation.setUserAgentOverride"));
        let metrics = &got
            .iter()
            .find(|(m, _)| *m == "Emulation.setDeviceMetricsOverride")
            .unwrap()
            .1;
        assert_eq!(metrics["width"], 1280);
        assert_eq!(metrics["deviceScaleFactor"], 1.0);
        assert_eq!(
            metrics["screenOrientation"],
            json!({"angle": 0, "type": "landscapePrimary"})
        );
    }

    #[test]
    fn headful_skips_font_families_and_no_viewport_skips_metrics() {
        let o = ContextOptions {
            viewport: None,
            ..ContextOptions::default()
        };
        let got = page_init_commands(&InitContext {
            options: &o,
            headless: false,
            font_platform: FontPlatform::Linux,
            window_id: Some(1),
            utility_world: WORLD,
        });
        let methods: Vec<_> = got.iter().map(|(m, _)| *m).collect();
        assert!(!methods.contains(&"Page.setFontFamilies"));
        assert!(!methods.contains(&"Emulation.setDeviceMetricsOverride"));
    }

    #[test]
    fn optional_overrides() {
        let mut o = ContextOptions {
            locale: Some("fr-FR".into()),
            geolocation: Some(crate::domain::emulation::Geolocation {
                latitude: 1.0,
                longitude: 2.0,
                accuracy: 0.0,
            }),
            offline: true,
            java_script_enabled: false,
            bypass_csp: true,
            ignore_https_errors: true,
            ..ContextOptions::default()
        };
        o.extra_http_headers.insert("x-a".into(), "1".into());
        let got = page_init_commands(&InitContext {
            options: &o,
            headless: true,
            font_platform: FontPlatform::Linux,
            window_id: None,
            utility_world: WORLD,
        });
        let find = |m: &str| got.iter().find(|(x, _)| *x == m).map(|(_, p)| p.clone());
        assert_eq!(
            find("Emulation.setUserAgentOverride").unwrap(),
            json!({"userAgent": "", "acceptLanguage": "fr-FR"})
        );
        assert_eq!(
            find("Emulation.setGeolocationOverride").unwrap(),
            json!({"latitude": 1.0, "longitude": 2.0, "accuracy": 0.0})
        );
        assert_eq!(
            find("Network.emulateNetworkConditions").unwrap(),
            json!({"offline": true, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1})
        );
        assert_eq!(
            find("Network.setExtraHTTPHeaders").unwrap(),
            json!({"headers": {"x-a": "1"}})
        );
        assert_eq!(
            find("Emulation.setScriptExecutionDisabled").unwrap(),
            json!({"value": true})
        );
        assert_eq!(find("Page.setBypassCSP").unwrap(), json!({"enabled": true}));
        assert_eq!(
            find("Security.setIgnoreCertificateErrors").unwrap(),
            json!({"ignore": true})
        );
    }

    #[test]
    fn resized_viewport_follows_recorded_traffic() {
        let o = probe_options();
        let got = viewport_commands(&o, Some(1), Some(Viewport::new(500, 400)));
        assert_eq!(
            got[0],
            (
                "Browser.setWindowBounds",
                json!({"windowId": 1, "bounds": {"width": 500, "height": 400}})
            )
        );
        assert_eq!(got[1].1["screenWidth"], 500);
        assert_eq!(got[1].1["height"], 400);
    }
}
