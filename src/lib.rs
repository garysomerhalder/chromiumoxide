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

/// Downloads Chrome for Testing / chrome-headless-shell.
#[cfg(feature = "fetcher")]
pub use chromiumoxide_fetcher as fetcher;

pub use application::actions::{Polling, SetContent, StyleTag, WaitForFunction};
pub use application::browser::{Browser, LaunchOptions};
pub use application::context::{BrowserContext, ContextEvent, PageWaiter};
pub use application::credentials::{Credential, Credentials, NewCredential};
pub use application::discovery::Discovered;
pub use application::download::Download;
pub use application::events::{ConsoleMessage, Dialog, EventWaiter, PageEvent};
pub use application::expect::{
    Assertion, Expect, LocatorAssertions, PageAssertions, ResponseAssertions, expect,
};
pub use application::filechooser::{FileChooser, FileChooserWaiter};
pub use application::frame::Frame;
pub use application::handle::JsHandle;
pub use application::har::RouteFromHar;
pub use application::input::{Keyboard, Mouse, Touchscreen};
pub use application::locator::{
    BoundingBox, Click, ClickOptions, Locator, SelectOption, WaitFor, WaitForState,
};
pub use application::navigation::{Navigation, WaitForUrl};
pub use application::network::{Request, Response, SecurityDetails, ServerAddr};
pub use application::page::{Goto, Page};
pub use application::pdf::Pdf;
pub use application::route::{
    Abort, Continue, FetchResponse, Fulfill, Route, RouteFetch, RouteRegistration,
};
pub use application::screenshot::Screenshot;
pub use application::scripts::{BindingSource, ScriptTag};
pub use application::services::Services;
pub use application::storage::WebStorage;
pub use application::websocket::{WebSocket, WebSocketEvent, WsPayload};
pub use application::websocket_route::{WebSocketRoute, WebSocketServer};
pub use application::worker::Worker;
pub use domain::console::{Location, PageError};
pub use domain::devices::{Device, device, device_names};
pub use domain::discovery::Source as DiscoverySource;
pub use domain::emulation::{
    ColorScheme, ContextOptions, Contrast, ForcedColors, Geolocation, HttpCredentials, Media,
    ReducedMotion, ServiceWorkers, Viewport,
};
pub use domain::error::{Error, Result};
pub use domain::frame::LifecycleEvent as WaitUntil;
pub use domain::har::{Har, HarNotFound, RecordHar};
pub use domain::input::MouseButton;
pub use domain::launch::BrowserKind;
pub use domain::network::{HeaderEntry, RequestSizes, Timing};
pub use domain::pdf::{Margin as PdfMargin, PdfOptions};
pub use domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale,
};
pub use domain::selectors::{Filter, RoleOptions, TextMatch};
pub use domain::storage::{
    ClearCookies, Cookie, NameValue, NewCookie, OriginState, SameSite, StorageState,
};
pub use domain::url_matcher::UrlMatcher;
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
