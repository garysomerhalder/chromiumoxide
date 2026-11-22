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
pub use application::aria::AriaAssertion;
pub use application::aria_snapshot::{AriaSnapshotMode, AriaSnapshotOptions};
pub use application::browser::{Browser, BrowserEvent, LaunchOptions};
pub use application::clock::Clock;
pub use application::context::{BrowserContext, ContextEvent, PageWaiter, WebError};
pub use application::coverage::{Coverage, CoverageOptions, CssCoverageEntry, JsCoverageEntry};
pub use application::credentials::{Credential, Credentials, NewCredential};
pub use application::debugger::{Debugger, PausedDetails};
pub use application::discovery::Discovered;
pub use application::download::Download;
pub use application::events::{ConsoleMessage, Dialog, EventWaiter, PageEvent};
pub use application::expect::{
    Assertion, Expect, LocatorAssertions, PageAssertions, ResponseAssertions, expect,
};
pub use application::extras::{BrowserType, CdpSessionHandle, EmulateMedia, WaitForNavigation};
pub use application::filechooser::{FileChooser, FileChooserWaiter};
pub use application::frame::{ContentOptions, Frame};
pub use application::handle::JsHandle;
pub use application::har::RouteFromHar;
pub use application::input::{Keyboard, Mouse, Touchscreen};
pub use application::locator::{
    BoundingBox, Click, ClickOptions, DropFile, DropPayload, Locator, SelectOption, WaitFor,
    WaitForState,
};
pub use application::navigation::{Navigation, WaitForUrl};
pub use application::network::{Request, Response, SecurityDetails, ServerAddr};
pub use application::page::{Goto, Page, PageCloseOptions};
pub use application::pdf::Pdf;
pub use application::request::{
    ApiRequest, ApiRequestContext, ApiRequestOptions, ApiResponse, ApiTracing,
};
pub use application::route::{
    Abort, Continue, FetchResponse, Fulfill, Route, RouteFetch, RouteRegistration,
};
pub use application::screencast::{
    ActionStyle, ActionTitlePosition, Overlay, Screencast, ScreencastFrame, ScreencastOptions,
    ShowActions,
};
pub use application::screenshot::Screenshot;
pub use application::scripts::{BindingSource, ScriptTag};
pub use application::selectors::Selectors;
pub use application::services::Services;
pub use application::snapshot::{ScreenshotAssertion, UpdateSnapshots};
pub use application::storage::{StorageStateOptions, WebStorage};
pub use application::tracing::{Tracing, TracingStart};
pub use application::video::Video;
pub use application::webmcp::WebMcp;
pub use application::websocket::{WebSocket, WebSocketEvent, WsPayload};
pub use application::websocket_route::{WebSocketRoute, WebSocketServer};
pub use application::worker::Worker;
pub use domain::clock::{ClockTime, Ticks};
pub use domain::console::{Location, PageError};
pub use domain::coverage::Range as CoverageRange;
pub use domain::devices::{Device, device, device_names};
pub use domain::discovery::Source as DiscoverySource;
pub use domain::emulation::{
    ColorScheme, ContextOptions, Contrast, ForcedColors, Geolocation, HttpCredentials, Media,
    ReducedMotion, ServiceWorkers, Viewport,
};
pub use domain::error::{Error, Result};
pub use domain::frame::LifecycleEvent as WaitUntil;
pub use domain::har::{Har, HarNotFound, RecordHar};
pub use domain::http::MultipartValue;
pub use domain::input::MouseButton;
pub use domain::launch::{BrowserKind, Proxy};
pub use domain::network::{HeaderEntry, RequestSizes, Timing};
pub use domain::pdf::{Margin as PdfMargin, PdfOptions};
pub use domain::screenshot::{
    Animations, Caret, ImageFormat, Rect, ScreenshotOptions, ScreenshotScale,
};
pub use domain::selectors::{Filter, RoleOptions, TextMatch};
pub use domain::source::SourceLocation;
pub use domain::storage::{
    ClearCookies, Cookie, NameValue, NewCookie, OriginState, SameSite, StorageState,
};
pub use domain::url_matcher::UrlMatcher;
pub use domain::video::RecordVideo;
pub use domain::webmcp::{WebMcpAnnotations, WebMcpTool};
pub use ports::Channel;

/// A launched browser serving clients (Playwright's `BrowserServer`).
pub struct BrowserServer {
    browser: Browser,
}

impl BrowserServer {
    /// The URL clients pass to [`Chromium::connect`].
    pub fn ws_endpoint(&self) -> String {
        self.browser.ws_endpoint().unwrap_or_default()
    }
    /// Closes the browser (and every client's pages).
    pub async fn close(&self) -> Result<()> {
        self.browser.close().await
    }
    /// Same as [`close`](Self::close): the process is ended either way.
    pub async fn kill(&self) -> Result<()> {
        self.browser.close().await
    }
}

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
            video: Arc::new(adapters::video::FfmpegEncoder),
            http: Arc::new(adapters::http::TcpHttpClient),
            bridge: Arc::new(adapters::cdp::bridge::WsBridge),
        }
    }

    /// A standalone `APIRequestContext` (Playwright's `request.newContext()`).
    pub async fn request_context(options: ApiRequestOptions) -> Result<ApiRequestContext> {
        Ok(ApiRequestContext::new(Self::services(), options))
    }

    /// Discovers and launches Chromium (Playwright's `chromium.launch`).
    pub async fn launch(options: LaunchOptions) -> Result<Browser> {
        Browser::launch(Self::services(), options).await
    }

    /// Playwright's `chromium.launchPersistentContext(userDataDir, options)`.
    pub async fn launch_persistent_context(
        user_data_dir: impl Into<std::path::PathBuf>,
        options: LaunchOptions,
        context: ContextOptions,
    ) -> Result<BrowserContext> {
        Browser::launch_persistent(Self::services(), user_data_dir.into(), options, context).await
    }

    /// The executable that [`Chromium::launch`] would run.
    pub fn executable_path(options: &LaunchOptions) -> Result<Discovered> {
        let s = Self::services();
        application::discovery::discover(&*s.fs, &*s.env, options.executable_path.as_deref())
    }

    /// Playwright's `browserType.launchServer()`: a browser other processes
    /// can [`connect`](Self::connect) to (its endpoint speaks CDP).
    pub async fn launch_server(options: LaunchOptions) -> Result<BrowserServer> {
        let browser = Self::launch(LaunchOptions {
            channel: Channel::WebSocket,
            ..options
        })
        .await?;
        Ok(BrowserServer { browser })
    }

    /// Playwright's `browserType.connect(wsEndpoint)`: a client of a
    /// [`BrowserServer`] (or of any browser's DevTools WebSocket). Closing it
    /// disconnects and leaves the browser running.
    pub async fn connect(ws_endpoint: &str) -> Result<Browser> {
        Self::connect_over_cdp(ws_endpoint).await
    }

    /// Connects to a running browser's DevTools WebSocket URL.
    pub async fn connect_over_cdp(ws_endpoint: &str) -> Result<Browser> {
        let transport = adapters::transport::websocket::connect(ws_endpoint).await?;
        let cdp = Arc::new(adapters::cdp::Connection::spawn(transport));
        Browser::connect(Self::services(), cdp, Some(ws_endpoint.to_owned())).await
    }
}
