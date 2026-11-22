//! Playwright's device descriptors (`playwright.devices`), version 1.65.1.
//!
//! Data from Playwright's `deviceDescriptorsSource.json`
//! (Copyright (c) Microsoft Corporation, Apache License 2.0).

use crate::domain::emulation::{ContextOptions, Viewport};

/// One device descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub name: String,
    pub user_agent: String,
    pub viewport: Viewport,
    pub screen: Option<Viewport>,
    pub device_scale_factor: f64,
    pub is_mobile: bool,
    pub has_touch: bool,
    /// `chromium`, `firefox` or `webkit` (informational for a CDP client).
    pub default_browser_type: String,
}

impl Device {
    /// Context options emulating this device (like `newContext({...devices[name]})`).
    pub fn context_options(&self) -> ContextOptions {
        ContextOptions {
            viewport: Some(self.viewport),
            screen: self.screen,
            device_scale_factor: Some(self.device_scale_factor),
            is_mobile: self.is_mobile,
            has_touch: self.has_touch,
            user_agent: Some(self.user_agent.clone()),
            ..ContextOptions::default()
        }
    }
}

fn table() -> &'static serde_json::Map<String, serde_json::Value> {
    static TABLE: std::sync::OnceLock<serde_json::Map<String, serde_json::Value>> =
        std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        serde_json::from_str(include_str!("data/devices.json")).expect("valid device table")
    })
}

fn size(v: &serde_json::Value) -> Option<Viewport> {
    Some(Viewport::new(
        v.get("width")?.as_u64()? as u32,
        v.get("height")?.as_u64()? as u32,
    ))
}

/// Looks a device up by its Playwright name, e.g. `"iPhone 15"`.
pub fn device(name: &str) -> Option<Device> {
    let d = table().get(name)?;
    Some(Device {
        name: name.to_owned(),
        user_agent: d.get("userAgent")?.as_str()?.to_owned(),
        viewport: size(d.get("viewport")?)?,
        screen: d.get("screen").and_then(size),
        device_scale_factor: d.get("deviceScaleFactor")?.as_f64()?,
        is_mobile: d.get("isMobile")?.as_bool()?,
        has_touch: d.get("hasTouch")?.as_bool()?,
        default_browser_type: d.get("defaultBrowserType")?.as_str()?.to_owned(),
    })
}

/// All device names, sorted.
pub fn device_names() -> Vec<String> {
    let mut names: Vec<String> = table().keys().cloned().collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iphone_15_matches_playwright() {
        let d = device("iPhone 15").unwrap();
        assert_eq!(d.viewport, Viewport::new(393, 659));
        assert_eq!(d.screen, Some(Viewport::new(393, 852)));
        assert_eq!(d.device_scale_factor, 3.0);
        assert!(d.is_mobile && d.has_touch);
        assert!(d.user_agent.contains("iPhone OS 17_5"));
        assert_eq!(d.default_browser_type, "webkit");
        // Playwright 1.65.1's descriptors (Safari 27.2 user agents).
        assert!(d.user_agent.contains("Version/27.2"), "{}", d.user_agent);
    }

    #[test]
    fn context_options_carry_the_descriptor() {
        let o = device("Pixel 7").unwrap().context_options();
        assert!(o.is_mobile && o.has_touch);
        assert!(o.user_agent.unwrap().contains("Android"));
        assert_eq!(o.device_scale_factor, Some(2.625));
        assert_eq!(o.viewport, Some(Viewport::new(412, 839)));
    }

    #[test]
    fn all_playwright_devices_are_present() {
        let names = device_names();
        assert_eq!(names.len(), 207);
        assert!(names.contains(&"Desktop Chrome".to_owned()));
        assert!(names.contains(&"iPad Pro 11 landscape".to_owned()));
        assert!(device("No Such Phone").is_none());
    }
}
