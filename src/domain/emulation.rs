//! Browser-context emulation options and the overrides derived from them,
//! following Playwright's Chromium implementation (`crPage.ts`).

use std::collections::BTreeMap;

/// A viewport or screen size in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// `prefers-color-scheme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
    NoPreference,
    /// Do not override.
    NoOverride,
}

/// `prefers-reduced-motion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReducedMotion {
    Reduce,
    #[default]
    NoPreference,
    NoOverride,
}

/// `forced-colors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForcedColors {
    Active,
    #[default]
    None,
    NoOverride,
}

/// `prefers-contrast`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Contrast {
    More,
    #[default]
    NoPreference,
    NoOverride,
}

/// CSS media type override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Media {
    Screen,
    Print,
    #[default]
    NoOverride,
}

/// HTTP basic/digest credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpCredentials {
    pub username: String,
    pub password: String,
    /// Only send to this origin (`scheme://host[:port]`), if set.
    pub origin: Option<String>,
}

/// Geolocation override.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geolocation {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy: f64,
}

/// Playwright's `browser.newContext(options)`.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextOptions {
    /// `None` is Playwright's `viewport: null` (no viewport emulation).
    pub viewport: Option<Viewport>,
    pub screen: Option<Viewport>,
    pub device_scale_factor: Option<f64>,
    pub is_mobile: bool,
    pub has_touch: bool,
    pub user_agent: Option<String>,
    pub locale: Option<String>,
    pub timezone_id: Option<String>,
    pub color_scheme: Option<ColorScheme>,
    pub reduced_motion: Option<ReducedMotion>,
    pub forced_colors: Option<ForcedColors>,
    pub contrast: Option<Contrast>,
    pub geolocation: Option<Geolocation>,
    pub permissions: Vec<String>,
    pub offline: bool,
    pub extra_http_headers: BTreeMap<String, String>,
    pub http_credentials: Option<HttpCredentials>,
    pub java_script_enabled: bool,
    pub bypass_csp: bool,
    pub ignore_https_errors: bool,
    pub base_url: Option<String>,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            viewport: Some(Viewport::new(1280, 720)),
            screen: None,
            device_scale_factor: None,
            is_mobile: false,
            has_touch: false,
            user_agent: None,
            locale: None,
            timezone_id: None,
            color_scheme: None,
            reduced_motion: None,
            forced_colors: None,
            contrast: None,
            geolocation: None,
            permissions: Vec::new(),
            offline: false,
            extra_http_headers: BTreeMap::new(),
            http_credentials: None,
            java_script_enabled: true,
            bypass_csp: false,
            ignore_https_errors: false,
            base_url: None,
        }
    }
}

/// Why a set of options is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidOptions(pub String);

/// Screen orientation reported to the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationType {
    PortraitPrimary,
    LandscapePrimary,
}

/// Screen orientation reported to the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenOrientation {
    pub angle: u32,
    pub kind: OrientationType,
}

/// What `Emulation.setDeviceMetricsOverride` should receive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceMetrics {
    pub mobile: bool,
    pub width: u32,
    pub height: u32,
    pub screen_width: u32,
    pub screen_height: u32,
    pub device_scale_factor: f64,
    pub orientation: ScreenOrientation,
}

/// User-agent client hints metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAgentMetadata {
    pub mobile: bool,
    pub model: String,
    pub architecture: String,
    pub platform: String,
    pub platform_version: String,
}

impl ContextOptions {
    /// Playwright's `validateBrowserContextOptions`.
    pub fn validate(&self) -> Result<(), InvalidOptions> {
        let fail = |m: &str| Err(InvalidOptions(m.to_owned()));
        if self.viewport.is_none() && self.device_scale_factor.is_some() {
            return fail(r#""deviceScaleFactor" option is not supported with null "viewport""#);
        }
        if self.viewport.is_none() && self.is_mobile {
            return fail(r#""isMobile" option is not supported with null "viewport""#);
        }
        if let Some(g) = &self.geolocation {
            if !(-180.0..=180.0).contains(&g.longitude) {
                return fail(
                    "geolocation.longitude: precondition -180 <= LONGITUDE <= 180 failed.",
                );
            }
            if !(-90.0..=90.0).contains(&g.latitude) {
                return fail("geolocation.latitude: precondition -90 <= LATITUDE <= 90 failed.");
            }
            if g.accuracy < 0.0 {
                return fail("geolocation.accuracy: precondition 0 <= ACCURACY failed.");
            }
        }
        Ok(())
    }

    /// Device metrics for a page with the given (possibly resized) viewport.
    /// `None` when viewport emulation is off.
    ///
    /// Like Playwright, once a page's viewport was resized the screen follows it.
    pub fn device_metrics(&self, resized: Option<Viewport>) -> Option<DeviceMetrics> {
        let (viewport, screen) = match resized {
            Some(v) => (v, v),
            None => {
                let v = self.viewport?;
                (v, self.screen.unwrap_or(v))
            }
        };
        let landscape = screen.width > screen.height;
        let orientation = match (self.is_mobile, landscape) {
            (true, true) => ScreenOrientation {
                angle: 90,
                kind: OrientationType::LandscapePrimary,
            },
            (true, false) => ScreenOrientation {
                angle: 0,
                kind: OrientationType::PortraitPrimary,
            },
            (false, _) => ScreenOrientation {
                angle: 0,
                kind: OrientationType::LandscapePrimary,
            },
        };
        Some(DeviceMetrics {
            mobile: self.is_mobile,
            width: viewport.width,
            height: viewport.height,
            screen_width: screen.width,
            screen_height: screen.height,
            device_scale_factor: self
                .device_scale_factor
                .filter(|d| *d != 0.0)
                .unwrap_or(1.0),
            orientation,
        })
    }

    /// UA client hints, derived from the user agent string (Playwright's
    /// `calculateUserAgentMetadata`).
    pub fn user_agent_metadata(&self) -> Option<UserAgentMetadata> {
        let ua = self.user_agent.as_deref()?;
        let mut m = UserAgentMetadata {
            mobile: self.is_mobile,
            model: String::new(),
            architecture: "x86".into(),
            platform: "Windows".into(),
            platform_version: String::new(),
        };
        let windows = ua.find("Windows").and_then(|i| {
            let rest = &ua[i + "Windows".len()..];
            let skip = rest.find(|c: char| c.is_ascii_digit())?;
            (skip > 0)
                .then(|| version_at(&rest[skip..], '.', 3))
                .flatten()
        });
        if let Some(v) = version_after(ua, "Android ", '.', 3) {
            (m.platform, m.platform_version, m.architecture) = ("Android".into(), v, "arm".into());
        } else if let Some(v) = version_after(ua, "iPhone OS ", '_', 2) {
            (m.platform, m.platform_version, m.architecture) = ("iOS".into(), v, "arm".into());
        } else if let Some(v) = version_after(ua, "iPad; CPU OS ", '_', 2) {
            (m.platform, m.platform_version, m.architecture) = ("iOS".into(), v, "arm".into());
        } else if let Some(v) = version_after(ua, "Mac OS X ", '_', 3) {
            (m.platform, m.platform_version) = ("macOS".into(), v);
            if !ua.contains("Intel") {
                m.architecture = "arm".into();
            }
        } else if let Some(v) = windows {
            (m.platform, m.platform_version) = ("Windows".into(), v);
        } else if ua.to_lowercase().contains("linux") {
            m.platform = "Linux".into();
        }
        if ua.contains("ARM") {
            m.architecture = "arm".into();
        }
        Some(m)
    }

    /// The CSS media type and media features for `Emulation.setEmulatedMedia`
    /// (an empty value means "no override").
    pub fn emulated_media(&self, media: Media) -> (String, Vec<(String, String)>) {
        let media = match media {
            Media::Screen => "screen",
            Media::Print => "print",
            Media::NoOverride => "",
        };
        let color = match self.color_scheme.unwrap_or_default() {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
            ColorScheme::NoPreference => "no-preference",
            ColorScheme::NoOverride => "",
        };
        let motion = match self.reduced_motion.unwrap_or_default() {
            ReducedMotion::Reduce => "reduce",
            ReducedMotion::NoPreference => "no-preference",
            ReducedMotion::NoOverride => "",
        };
        let forced = match self.forced_colors.unwrap_or_default() {
            ForcedColors::Active => "active",
            ForcedColors::None => "none",
            ForcedColors::NoOverride => "",
        };
        let contrast = match self.contrast.unwrap_or_default() {
            Contrast::More => "more",
            Contrast::NoPreference => "no-preference",
            Contrast::NoOverride => "",
        };
        let features = [
            ("prefers-color-scheme", color),
            ("prefers-reduced-motion", motion),
            ("forced-colors", forced),
            ("prefers-contrast", contrast),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        (media.to_owned(), features)
    }
}

/// The version right after the first `prefix` in `s`.
fn version_after(s: &str, prefix: &str, sep: char, max_parts: usize) -> Option<String> {
    let i = s.find(prefix)?;
    version_at(&s[i + prefix.len()..], sep, max_parts)
}

/// Parses `\d+(sep\d+){0,max_parts-1}` at the start of `s`.
fn version_at(s: &str, sep: char, max_parts: usize) -> Option<String> {
    let mut out = String::new();
    let mut rest = s;
    for part in 0..max_parts {
        let body = if part == 0 {
            rest
        } else {
            rest.strip_prefix(sep).unwrap_or("")
        };
        let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            break;
        }
        if part > 0 {
            out.push(sep);
        }
        out.push_str(&body[..digits]);
        rest = &body[digits..];
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ContextOptions {
        ContextOptions::default()
    }

    #[test]
    fn default_viewport_is_1280_by_720() {
        assert_eq!(opts().viewport, Some(Viewport::new(1280, 720)));
    }

    #[test]
    fn device_scale_factor_and_mobile_need_a_viewport() {
        let o = ContextOptions {
            viewport: None,
            device_scale_factor: Some(2.0),
            ..opts()
        };
        assert_eq!(
            o.validate().unwrap_err().0,
            r#""deviceScaleFactor" option is not supported with null "viewport""#
        );
        let o = ContextOptions {
            viewport: None,
            is_mobile: true,
            ..opts()
        };
        assert_eq!(
            o.validate().unwrap_err().0,
            r#""isMobile" option is not supported with null "viewport""#
        );
        assert!(opts().validate().is_ok());
    }

    #[test]
    fn geolocation_ranges_are_checked() {
        let o = ContextOptions {
            geolocation: Some(Geolocation {
                latitude: 0.0,
                longitude: 200.0,
                accuracy: 0.0,
            }),
            ..opts()
        };
        assert_eq!(
            o.validate().unwrap_err().0,
            "geolocation.longitude: precondition -180 <= LONGITUDE <= 180 failed."
        );
        let o = ContextOptions {
            geolocation: Some(Geolocation {
                latitude: -91.0,
                longitude: 0.0,
                accuracy: 0.0,
            }),
            ..opts()
        };
        assert_eq!(
            o.validate().unwrap_err().0,
            "geolocation.latitude: precondition -90 <= LATITUDE <= 90 failed."
        );
        let o = ContextOptions {
            geolocation: Some(Geolocation {
                latitude: 0.0,
                longitude: 0.0,
                accuracy: -1.0,
            }),
            ..opts()
        };
        assert_eq!(
            o.validate().unwrap_err().0,
            "geolocation.accuracy: precondition 0 <= ACCURACY failed."
        );
    }

    #[test]
    fn desktop_metrics_match_playwright() {
        let m = opts().device_metrics(None).unwrap();
        assert_eq!(
            m,
            DeviceMetrics {
                mobile: false,
                width: 1280,
                height: 720,
                screen_width: 1280,
                screen_height: 720,
                device_scale_factor: 1.0,
                orientation: ScreenOrientation {
                    angle: 0,
                    kind: OrientationType::LandscapePrimary
                },
            }
        );
    }

    #[test]
    fn mobile_landscape_metrics_match_recorded_playwright_traffic() {
        // From a recorded Playwright 1.63 session: viewport 400x300, dsf 2, isMobile.
        let o = ContextOptions {
            viewport: Some(Viewport::new(400, 300)),
            device_scale_factor: Some(2.0),
            is_mobile: true,
            ..opts()
        };
        let m = o.device_metrics(None).unwrap();
        assert_eq!(
            (m.width, m.height, m.screen_width, m.screen_height),
            (400, 300, 400, 300)
        );
        assert_eq!(m.device_scale_factor, 2.0);
        assert_eq!(
            m.orientation,
            ScreenOrientation {
                angle: 90,
                kind: OrientationType::LandscapePrimary
            }
        );
        // After page.setViewportSize(500x400) the screen follows the viewport.
        let m = o.device_metrics(Some(Viewport::new(500, 400))).unwrap();
        assert_eq!(
            (m.width, m.height, m.screen_width, m.screen_height),
            (500, 400, 500, 400)
        );
    }

    #[test]
    fn mobile_portrait_and_explicit_screen() {
        let o = ContextOptions {
            viewport: Some(Viewport::new(390, 664)),
            screen: Some(Viewport::new(390, 844)),
            device_scale_factor: Some(3.0),
            is_mobile: true,
            ..opts()
        };
        let m = o.device_metrics(None).unwrap();
        assert_eq!((m.screen_width, m.screen_height), (390, 844));
        assert_eq!(
            m.orientation,
            ScreenOrientation {
                angle: 0,
                kind: OrientationType::PortraitPrimary
            }
        );
        assert!(
            ContextOptions {
                viewport: None,
                ..opts()
            }
            .device_metrics(None)
            .is_none()
        );
    }

    fn ua(ua: &str, mobile: bool) -> UserAgentMetadata {
        ContextOptions {
            user_agent: Some(ua.into()),
            is_mobile: mobile,
            ..opts()
        }
        .user_agent_metadata()
        .unwrap()
    }

    #[test]
    fn user_agent_metadata_like_playwright() {
        assert_eq!(ContextOptions::default().user_agent_metadata(), None);
        let m = ua("UA-X", true);
        assert_eq!(
            (
                m.mobile,
                m.platform.as_str(),
                m.architecture.as_str(),
                m.platform_version.as_str()
            ),
            (true, "Windows", "x86", "")
        );
        let m = ua(
            "Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36",
            true,
        );
        assert_eq!(
            (
                m.platform.as_str(),
                m.platform_version.as_str(),
                m.architecture.as_str()
            ),
            ("Android", "14", "arm")
        );
        let m = ua(
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15",
            true,
        );
        assert_eq!(
            (
                m.platform.as_str(),
                m.platform_version.as_str(),
                m.architecture.as_str()
            ),
            ("iOS", "17_5", "arm")
        );
        let m = ua("Mozilla/5.0 (iPad; CPU OS 16_0 like Mac OS X)", true);
        assert_eq!(
            (m.platform.as_str(), m.platform_version.as_str()),
            ("iOS", "16_0")
        );
        let m = ua(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36",
            false,
        );
        assert_eq!(
            (
                m.platform.as_str(),
                m.platform_version.as_str(),
                m.architecture.as_str()
            ),
            ("macOS", "10_15_7", "x86")
        );
        let m = ua("Mozilla/5.0 (Macintosh; Mac OS X 14_0)", false);
        assert_eq!(m.architecture, "arm");
        let m = ua("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", false);
        assert_eq!(
            (m.platform.as_str(), m.platform_version.as_str()),
            ("Windows", "10.0")
        );
        let m = ua("Mozilla/5.0 (X11; Linux x86_64)", false);
        assert_eq!(m.platform, "Linux");
        let m = ua("Mozilla/5.0 (X11; Linux ARM)", false);
        assert_eq!(m.architecture, "arm");
    }

    fn feats(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| ((*a).into(), (*b).into())).collect()
    }

    #[test]
    fn emulated_media_defaults_to_light_like_playwright() {
        assert_eq!(
            opts().emulated_media(Media::NoOverride),
            (
                String::new(),
                feats(&[
                    ("prefers-color-scheme", "light"),
                    ("prefers-reduced-motion", "no-preference"),
                    ("forced-colors", "none"),
                    ("prefers-contrast", "no-preference"),
                ])
            )
        );
    }

    #[test]
    fn emulated_media_overrides() {
        let o = ContextOptions {
            color_scheme: Some(ColorScheme::Dark),
            reduced_motion: Some(ReducedMotion::Reduce),
            forced_colors: Some(ForcedColors::Active),
            contrast: Some(Contrast::NoOverride),
            ..opts()
        };
        assert_eq!(
            o.emulated_media(Media::Print),
            (
                "print".into(),
                feats(&[
                    ("prefers-color-scheme", "dark"),
                    ("prefers-reduced-motion", "reduce"),
                    ("forced-colors", "active"),
                    ("prefers-contrast", ""),
                ])
            )
        );
    }
}

/// Browser platform, as Playwright infers it from the browser's user agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontPlatform {
    Linux,
    Mac,
    Win,
}

impl FontPlatform {
    /// Playwright's `CRBrowser._platform()`.
    pub fn from_user_agent(_ua: &str) -> Self {
        unimplemented!()
    }
}

/// Playwright's `platformToFontFamilies` entry: the `Page.setFontFamilies`
/// parameters applied to every headless page.
pub fn default_font_families(_platform: FontPlatform) -> serde_json::Value {
    unimplemented!()
}

#[cfg(test)]
mod font_tests {
    use super::*;

    #[test]
    fn platform_from_browser_user_agent() {
        assert_eq!(
            FontPlatform::from_user_agent("Mozilla/5.0 (X11; Linux x86_64) HeadlessChrome/157"),
            FontPlatform::Linux
        );
        assert_eq!(
            FontPlatform::from_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"),
            FontPlatform::Mac
        );
        assert_eq!(
            FontPlatform::from_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"),
            FontPlatform::Win
        );
    }

    #[test]
    fn linux_families_match_recorded_playwright_traffic() {
        assert_eq!(
            default_font_families(FontPlatform::Linux),
            serde_json::json!({"fontFamilies": {"standard": "Times New Roman", "fixed": "Monospace", "serif": "Times New Roman", "sansSerif": "Arial", "cursive": "Comic Sans MS", "fantasy": "Impact"}})
        );
    }

    #[test]
    fn mac_families_include_per_script_fonts() {
        let mac = default_font_families(FontPlatform::Mac);
        assert_eq!(mac["fontFamilies"]["sansSerif"], "Helvetica");
        assert!(mac["forScripts"].as_array().is_some_and(|a| !a.is_empty()));
        assert_eq!(
            default_font_families(FontPlatform::Win)["fontFamilies"]["standard"],
            "Times New Roman"
        );
    }
}
