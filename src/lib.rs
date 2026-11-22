//! chromiumoxide: a Chrome DevTools Protocol client with a Playwright-like API.
//!
//! ```no_run
//! # async fn run() -> chromiumoxide::Result<()> {
//! use chromiumoxide::{Chromium, LaunchOptions};
//! let browser = Chromium::launch(LaunchOptions::default()).await?;
//! println!("{}", browser.version().await?);
//! browser.close().await?;
//! # Ok(()) }
//! ```
//!
//! See `docs/ARCHITECTURE.md` for the layering.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;

#[cfg(test)]
mod testing;

use std::sync::Arc;

/// The generated Chrome DevTools Protocol types.
pub use chromiumoxide_cdp::cdp;

pub use application::actions::{Polling, SetContent, StyleTag, WaitForFunction};
pub use application::browser::{Browser, LaunchOptions};
pub use application::context::BrowserContext;
pub use application::discovery::Discovered;
pub use application::page::{Goto, Page, Response};
pub use application::screenshot::Screenshot;
pub use application::services::Services;
pub use domain::devices::{Device, device, device_names};
pub use domain::discovery::Source as DiscoverySource;
pub use domain::emulation::{
    ColorScheme, ContextOptions, Contrast, ForcedColors, Geolocation, HttpCredentials, Media,
    ReducedMotion, Viewport,
};
pub use domain::error::{Error, Result};
pub use domain::frame::LifecycleEvent as WaitUntil;
pub use domain::launch::BrowserKind;
pub use domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale,
};
pub use ports::Channel;

/// Entry point, like Playwright's `chromium` browser type.
pub struct Chromium;

impl Chromium {
    /// The production port implementations.
    pub fn services() -> Services {
        Services {
            fs: Arc::new(adapters::os::OsFileSystem),
            env: Arc::new(adapters::os::OsEnvironment),
            launcher: Arc::new(adapters::process::TokioLauncher),
            clock: Arc::new(adapters::clock::TokioClock),
        }
    }

    /// Discovers and launches Chromium (Playwright's `chromium.launch`).
    pub async fn launch(options: LaunchOptions) -> Result<Browser> {
        Browser::launch(Self::services(), options).await
    }

    /// The executable that [`Chromium::launch`] would run.
    pub fn executable_path(options: &LaunchOptions) -> Result<Discovered> {
        let s = Self::services();
        application::discovery::discover(&*s.fs, &*s.env, options.executable_path.as_deref())
    }

    /// Connects to a running browser's DevTools WebSocket URL.
    pub async fn connect_over_cdp(ws_endpoint: &str) -> Result<Browser> {
        let transport = adapters::transport::websocket::connect(ws_endpoint).await?;
        let cdp = Arc::new(adapters::cdp::Connection::spawn(transport));
        Browser::connect(Self::services(), cdp, Some(ws_endpoint.to_owned())).await
    }
}
