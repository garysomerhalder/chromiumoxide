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
pub use application::android::{
    AndroidDevice, AndroidDeviceEvent, AndroidDevicesOptions, AndroidLaunchOptions, AndroidSocket,
    AndroidWebView,
};
pub use application::aria::AriaAssertion;
pub use application::aria_snapshot::{AriaSnapshotMode, AriaSnapshotOptions};
pub use application::browser::{BindOptions, Browser, BrowserEvent, LaunchOptions};
pub use application::clock::Clock;
pub use application::context::{BrowserContext, ContextEvent, PageWaiter, WebError};
pub use application::coverage::{Coverage, CoverageOptions, CssCoverageEntry, JsCoverageEntry};
pub use application::credentials::{Credential, Credentials, NewCredential};
pub use application::debugger::{Debugger, PausedDetails};
pub use application::discovery::Discovered;
pub use application::download::Download;
pub use application::electron::{
    ElectronApplication, ElectronConsoleMessage, ElectronEvent, ElectronHandle,
    ElectronLaunchOptions, ElectronProcess,
};
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
pub use application::selectors::{SelectorRegisterOptions, Selectors};
pub use application::services::Services;
pub use application::snapshot::{ScreenshotAssertion, UpdateSnapshots};
pub use application::storage::{StorageStateOptions, WebStorage};
pub use application::tracing::{Tracing, TracingStart};
pub use application::video::Video;
pub use application::webmcp::WebMcp;
pub use application::websocket::{WebSocket, WebSocketEvent, WsPayload};
pub use application::websocket_route::{WebSocketRoute, WebSocketServer};
pub use application::worker::{Worker, WorkerEvent};
pub use domain::client_cert::ClientCertificate;
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
pub use domain::har::{Har, HarContent, HarMode, HarNotFound, HarOptions, RecordHar};
pub use domain::http::MultipartValue;
pub use domain::input::MouseButton;
pub use domain::launch::{BrowserKind, Proxy};
pub use domain::network::{HeaderEntry, RequestSizes, Timing};
pub use domain::pdf::{Margin as PdfMargin, PdfOptions};
pub use domain::registry::BoundBrowser;
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

/// Playwright's `_android`: Android devices through the adb server.
pub struct Android;

impl Android {
    /// `android.devices(options)`.
    pub async fn devices(options: AndroidDevicesOptions) -> Result<Vec<AndroidDevice>> {
        application::android::devices(Chromium::services(), options).await
    }
}

/// The `chrome-command-line` Playwright writes for Chrome on Android.
pub fn android_chrome_command_line(args: &[String], socket_name: &str) -> String {
    domain::android::chrome_command_line(args, socket_name)
}

/// The Firefox engine (WebDriver BiDi): its `Browser`, `Page`, ... have the
/// Playwright method names of the Chromium ones.
pub mod firefox {
    pub use crate::application::firefox::{
        Browser, BrowserContext, FirefoxLaunchOptions, Frame, Page, Screenshot,
    };
    pub use crate::application::firefox_binding::BindingSource;
    pub use crate::application::firefox_clock::Clock;
    pub use crate::application::firefox_context_more::BrowserType;
    pub use crate::application::firefox_download::Download;
    pub use crate::application::firefox_events::FileChooser;
    pub use crate::application::firefox_events::{
        ConsoleMessage, ContextEvent, Dialog, PageError, PageEvent,
    };
    pub use crate::application::firefox_expect::{Assertion, LocatorAssertions, PageAssertions};
    pub use crate::application::firefox_handle::{ElementHandle, JsHandle};
    pub use crate::application::firefox_input::{Keyboard, Mouse};
    pub use crate::application::firefox_locator::{FrameLocator, Locator};
    pub use crate::application::firefox_page_more::Touchscreen;
    pub use crate::application::firefox_record::{Pdf, Tracing, Video};
    pub use crate::application::firefox_route::{Fulfill, Request, Response, Route, RouteRequest};
    pub use crate::application::firefox_worker::{Worker, WorkerEvent};
    pub use crate::application::websocket::{WebSocket, WebSocketEvent, WsPayload};
}

/// The WebKit engine (`WebKit::launch`): Playwright's WebKit build over its
/// own protocol, with the Playwright method names.
pub mod webkit {
    pub use crate::application::webkit::{
        Browser, BrowserContext, Page, Screenshot, WebKitLaunchOptions,
    };
    pub use crate::application::webkit_binding::BindingSource;
    pub use crate::application::webkit_clock::Clock;
    pub use crate::application::webkit_download::Download;
    pub use crate::application::webkit_events::{
        ConsoleMessage, ContextEvent, Dialog, FileChooser, PageError, PageEvent,
    };
    pub use crate::application::webkit_expect::{Assertion, LocatorAssertions, PageAssertions};
    pub use crate::application::webkit_frame::{Frame, FrameLocator};
    pub use crate::application::webkit_handle::{ElementHandle, JsHandle};
    pub use crate::application::webkit_har::RouteFromHar;
    pub use crate::application::webkit_locator::Locator;
    pub use crate::application::webkit_network::{Request, Response};
    pub use crate::application::webkit_page_more::{BrowserType, Keyboard, Mouse, Touchscreen};
    pub use crate::application::webkit_record::Video;
    pub use crate::application::webkit_route::{Fulfill, Route};
    pub use crate::application::webkit_trace::Tracing;
    pub use crate::application::webkit_worker::{Worker, WorkerEvent};
}

/// Playwright's `webkit`: launches Playwright's WebKit build (Safari's
/// engine).
pub struct WebKit;

impl WebKit {
    /// `webkit.launch(options)`.
    pub async fn launch(options: webkit::WebKitLaunchOptions) -> Result<webkit::Browser> {
        webkit::Browser::launch(Chromium::services(), options).await
    }

    /// `webkit.launchServer(options)`: a WebKit other programs can
    /// [`connect`](Self::connect) to, on a free local port.
    pub async fn launch_server(options: webkit::WebKitLaunchOptions) -> Result<WebKitServer> {
        Self::launch_server_with(options, ServerAddress::default()).await
    }

    /// `webkit.launchServer({ host, port, wsPath })`.
    pub async fn launch_server_with(
        options: webkit::WebKitLaunchOptions,
        address: ServerAddress,
    ) -> Result<WebKitServer> {
        let browser = Self::launch(options).await?;
        let served = browser
            .serve(ports::BridgeAddress {
                host: address.host,
                port: address.port,
                path: address.ws_path,
            })
            .await;
        match served {
            Ok(endpoint) => Ok(WebKitServer { browser, endpoint }),
            Err(e) => {
                let _ = browser.close().await;
                Err(e)
            }
        }
    }

    /// Connects to a WebKit shared with `browser.bind()` (its endpoint).
    /// Closing it disconnects and leaves the browser running.
    pub async fn connect(ws_endpoint: &str) -> Result<webkit::Browser> {
        let transport = adapters::transport::websocket::connect(ws_endpoint).await?;
        let wk = std::sync::Arc::new(adapters::webkit::WebKitConnection::spawn(transport));
        webkit::Browser::connect(Chromium::services(), wk).await
    }
}

pub use crate::application::extension_pages::{PageBrowser, PageTab};
pub use crate::application::safari::{SafariBrowser, SafariLaunchOptions};
pub use crate::application::tauri::{
    TauriApp, TauriLaunchOptions, TauriLocator, TauriWindow, WebDriverLocator, WebDriverWindow,
};

/// Safari, through Apple's `safaridriver` (WebDriver). Real Safari runs on
/// macOS only; Playwright's WebKit ([`WebKit`]) runs its engine everywhere.
pub struct Safari;

impl Safari {
    /// Starts `safaridriver` and opens a Safari automation window.
    pub async fn launch(options: SafariLaunchOptions) -> Result<SafariBrowser> {
        SafariBrowser::launch(Chromium::services(), options).await
    }
}

/// Tauri apps: on Windows their WebView2 windows are driven over CDP with
/// the Chromium API.
pub struct Tauri;

impl Tauri {
    /// Starts a Tauri app and connects to its web view.
    pub async fn launch(options: TauriLaunchOptions) -> Result<TauriApp> {
        TauriApp::launch(Chromium::services(), options).await
    }
}

/// Playwright's `firefox`: launches Firefox, driven over WebDriver BiDi.
pub struct Firefox;

impl Firefox {
    /// `firefox.launch(options)`.
    pub async fn launch(options: firefox::FirefoxLaunchOptions) -> Result<firefox::Browser> {
        firefox::Browser::launch(Chromium::services(), options).await
    }

    /// `firefox.launchPersistentContext(userDataDir, options)`: the profile
    /// in `user_data_dir` is kept between launches.
    pub async fn launch_persistent_context(
        user_data_dir: impl AsRef<std::path::Path>,
        options: firefox::FirefoxLaunchOptions,
        context_options: ContextOptions,
    ) -> Result<firefox::BrowserContext> {
        firefox::Browser::launch_persistent_context(
            Chromium::services(),
            user_data_dir.as_ref(),
            options,
            context_options,
        )
        .await
    }

    /// `firefox.launchServer(options)`: a Firefox other programs can
    /// [`connect`](Self::connect) to, on a free local port. Firefox allows
    /// one WebDriver BiDi session, so clients share it through a relay.
    pub async fn launch_server(options: firefox::FirefoxLaunchOptions) -> Result<FirefoxServer> {
        Self::launch_server_with(options, ServerAddress::default()).await
    }

    /// `firefox.launchServer({ host, port, wsPath })`.
    pub async fn launch_server_with(
        options: firefox::FirefoxLaunchOptions,
        address: ServerAddress,
    ) -> Result<FirefoxServer> {
        let browser = Self::launch(options).await?;
        let served = browser
            .serve(ports::BridgeAddress {
                host: address.host,
                port: address.port,
                path: address.ws_path,
            })
            .await;
        match served {
            Ok(endpoint) => Ok(FirefoxServer { browser, endpoint }),
            Err(e) => {
                let _ = browser.close().await;
                Err(e)
            }
        }
    }

    /// `firefox.connect(wsEndpoint)`: a running Firefox's WebDriver BiDi
    /// endpoint (`ws://127.0.0.1:<port>` as it prints with
    /// `--remote-debugging-port`, or its `/session` URL).
    pub async fn connect(ws_endpoint: &str) -> Result<firefox::Browser> {
        let base = ws_endpoint.trim_end_matches('/');
        let url = if base.ends_with("/session") {
            base.to_owned()
        } else {
            format!("{base}/session")
        };
        let transport = adapters::transport::websocket::connect(&url).await?;
        let bidi = std::sync::Arc::new(adapters::bidi::BidiConnection::spawn(transport));
        firefox::Browser::connect(Chromium::services(), bidi).await
    }
}

/// Playwright's `_electron`: launches Electron apps.
pub struct Electron;

impl Electron {
    /// `electron.launch(options)`.
    pub async fn launch(options: ElectronLaunchOptions) -> Result<ElectronApplication> {
        let found = if options.executable_path.is_none() {
            Self::find_executable()
        } else {
            None
        };
        ElectronApplication::launch(Chromium::services(), options, found).await
    }

    /// The Electron binary `require('electron')` would use:
    /// `ELECTRON_OVERRIDE_DIST_PATH`, else `node_modules/electron` in the
    /// working directory or a parent.
    pub fn find_executable() -> Option<std::path::PathBuf> {
        adapters::electron::find_executable()
    }
}

/// A launched browser serving clients (Playwright's `BrowserServer`).
pub struct BrowserServer {
    browser: Browser,
    /// The bridge endpoint, when served at a chosen address.
    endpoint: Option<String>,
}

/// Playwright's `browserType.launchServer(options)`: launch options plus
/// where to listen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchServerOptions {
    pub launch: LaunchOptions,
    /// Default `127.0.0.1`.
    pub host: Option<String>,
    /// Default `0`: any free port.
    pub port: Option<u16>,
    /// Playwright's `wsPath`: the only path clients may connect to. Anyone
    /// who knows it can drive the browser; keep it unguessable.
    pub ws_path: Option<String>,
}

/// `browserServer.process()`: the launched browser process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerProcess {
    pid: u32,
}

impl ServerProcess {
    /// The operating-system process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl BrowserServer {
    /// `browserServer.process()`: the browser's process (`None` once closed).
    pub fn process(&self) -> Option<ServerProcess> {
        self.browser.process_id().map(|pid| ServerProcess { pid })
    }
    /// The URL clients pass to [`Chromium::connect`].
    pub fn ws_endpoint(&self) -> String {
        self.endpoint
            .clone()
            .or_else(|| self.browser.ws_endpoint())
            .unwrap_or_default()
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

/// Where `launch_server_with` listens (Playwright's `host`, `port`,
/// `wsPath`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerAddress {
    /// Default `127.0.0.1`.
    pub host: Option<String>,
    /// Default `0`: any free port.
    pub port: Option<u16>,
    /// The only path clients may connect to. Anyone who knows it can drive
    /// the browser; keep it unguessable.
    pub ws_path: Option<String>,
}

macro_rules! engine_server {
    ($name:ident, $browser:ty, $connect:literal) => {
        #[doc = concat!("A launched browser serving clients (Playwright's `BrowserServer`); they connect with `", $connect, "`.")]
        pub struct $name {
            browser: $browser,
            endpoint: String,
        }

        impl $name {
            /// `browserServer.process()`: the browser's process (`None` once closed).
            pub fn process(&self) -> Option<ServerProcess> {
                self.browser.process_id().map(|pid| ServerProcess { pid })
            }
            #[doc = concat!("`browserServer.wsEndpoint()`: the URL clients pass to `", $connect, "`.")]
            pub fn ws_endpoint(&self) -> String {
                self.endpoint.clone()
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
    };
}

engine_server!(FirefoxServer, firefox::Browser, "Firefox::connect");
engine_server!(WebKitServer, webkit::Browser, "WebKit::connect");

/// `Chromium::launch_with_extension`: an ordinary Chrome or Edge with the
/// extension, driven only through it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtensionLaunchOptions {
    /// Chrome, Edge or Chromium; `None` finds one as `Chromium::launch` does.
    pub executable_path: Option<std::path::PathBuf>,
    /// The unpacked extension (`extension/.output/chrome-mv3` or `edge-mv3`).
    pub extension_path: std::path::PathBuf,
    /// The profile, kept between launches (logins stay).
    pub user_data_dir: std::path::PathBuf,
    /// Default `true`.
    pub headless: bool,
    pub args: Vec<String>,
    /// How long to wait for the extension to connect. Default 30 s.
    pub timeout: std::time::Duration,
}

impl Default for ExtensionLaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            extension_path: std::path::PathBuf::new(),
            user_data_dir: std::path::PathBuf::new(),
            headless: true,
            args: Vec::new(),
            timeout: std::time::Duration::from_secs(30),
        }
    }
}

/// Where the extension relay listens, and how long it waits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionRelayOptions {
    /// Default `127.0.0.1`.
    pub host: Option<String>,
    /// Default `0`: any free port.
    pub port: Option<u16>,
    /// How long [`ExtensionRelay::wait`] waits for the extension. Default 60 s.
    pub timeout: std::time::Duration,
}

impl Default for ExtensionRelayOptions {
    fn default() -> Self {
        Self {
            host: None,
            port: None,
            timeout: std::time::Duration::from_secs(60),
        }
    }
}

/// A relay waiting for the browser extension (`extension/`): opening
/// [`connect_url`](Self::connect_url) in a browser that has the extension
/// connects that browser.
pub struct ExtensionRelay {
    listener: adapters::extension_relay::Listener,
    timeout: std::time::Duration,
}

impl ExtensionRelay {
    /// The page to open in the browser that has the extension.
    pub fn connect_url(&self) -> String {
        self.listener.connect_url().to_owned()
    }

    /// Waits for the extension in Chrome or Edge, then returns its browser
    /// with the whole page API: `contexts()[0]` is the browser's own profile
    /// with its open tabs. Closing the returned browser disconnects and
    /// leaves the browser running. Firefox and Safari: [`wait_pages`](Self::wait_pages).
    pub async fn wait(self) -> Result<Browser> {
        let (transport, hello) = self.connected().await?;
        if hello.protocol != "cdp" {
            return Err(Error::InvalidArgument(format!(
                "the extension is in a browser without a debugger for extensions ({}); use wait_pages()",
                hello.user_agent
            )));
        }
        let cdp = Arc::new(adapters::cdp::Connection::spawn(transport));
        Browser::connect_own_profile(Chromium::services(), cdp).await
    }

    /// Waits for the extension in Firefox or Safari (whose extensions get no
    /// debugger): tabs, `goto`, `evaluate`, click, fill and screenshots.
    pub async fn wait_pages(self) -> Result<PageBrowser> {
        let (transport, hello) = self.connected().await?;
        if hello.protocol != "pages" {
            return Err(Error::InvalidArgument(
                "the extension is in Chrome or Edge; use wait() for the whole page API".into(),
            ));
        }
        let session = Arc::new(adapters::cdp::Connection::spawn(transport));
        Ok(PageBrowser::new(session, hello.user_agent))
    }

    /// The extension's socket, after its first message (`Extension.hello`).
    async fn connected(self) -> Result<(ports::Transport, Hello)> {
        use futures::StreamExt;
        let mut transport = self.listener.accept(self.timeout).await?;
        let first = transport
            .stream
            .next()
            .await
            .ok_or_else(|| Error::Transport("the extension closed the connection".into()))?
            .map_err(|e| Error::Transport(e.to_string()))?;
        let v: serde_json::Value = serde_json::from_str(&first)?;
        if v["method"] != "Extension.hello" {
            return Err(Error::Transport(format!(
                "the extension did not say hello: {first}"
            )));
        }
        let hello = Hello {
            protocol: v["params"]["protocol"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            user_agent: v["params"]["userAgent"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        };
        Ok((transport, hello))
    }
}

/// The extension's first message.
struct Hello {
    protocol: String,
    user_agent: String,
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
            bidi_bridge: Arc::new(adapters::bidi_bridge::WsBidiBridge),
            webkit_bridge: Arc::new(adapters::webkit_bridge::WsWebKitBridge),
            signals: Arc::new(adapters::signals::OsSignals),
            adb: Arc::new(adapters::adb::TcpAdb),
        }
    }

    /// A standalone `APIRequestContext` (Playwright's `request.newContext()`).
    pub async fn request_context(options: ApiRequestOptions) -> Result<ApiRequestContext> {
        domain::client_cert::validate(&options.client_certificates)
            .map_err(Error::InvalidArgument)?;
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
        Ok(BrowserServer {
            browser,
            endpoint: None,
        })
    }

    /// `browserType.launchServer({ host, port, wsPath })`: serves the browser
    /// at that address (and only that path); without any of them, like
    /// [`launch_server`](Self::launch_server).
    pub async fn launch_server_with(options: LaunchServerOptions) -> Result<BrowserServer> {
        if options.host.is_none() && options.port.is_none() && options.ws_path.is_none() {
            return Self::launch_server(options.launch).await;
        }
        let browser = Self::launch(options.launch).await?;
        let served = browser
            .serve(ports::BridgeAddress {
                host: options.host,
                port: options.port,
                path: options.ws_path,
            })
            .await;
        match served {
            Ok(endpoint) => Ok(BrowserServer {
                browser,
                endpoint: Some(endpoint),
            }),
            Err(e) => {
                let _ = browser.close().await;
                Err(e)
            }
        }
    }

    /// The browsers bound on this machine with
    /// [`Browser::bind_with`] (Playwright's server registry).
    pub async fn bound_browsers() -> Result<Vec<BoundBrowser>> {
        Browser::bound_browsers(&Self::services()).await
    }

    /// Playwright's `browserType.connect(wsEndpoint)`: a client of a
    /// [`BrowserServer`] (or of any browser's DevTools WebSocket). Closing it
    /// disconnects and leaves the browser running.
    pub async fn connect(ws_endpoint: &str) -> Result<Browser> {
        Self::connect_over_cdp(ws_endpoint).await
    }

    /// Listens for the browser extension (`extension/`), without opening
    /// anything: open [`ExtensionRelay::connect_url`] in the browser, then
    /// [`ExtensionRelay::wait`].
    pub async fn extension_relay(options: ExtensionRelayOptions) -> Result<ExtensionRelay> {
        let listener =
            adapters::extension_relay::listen(options.host.as_deref(), options.port).await?;
        Ok(ExtensionRelay {
            listener,
            timeout: options.timeout,
        })
    }

    /// Starts Chrome or Edge as an ordinary browser (no debugging port, no
    /// automation switches) with the extension and the profile in
    /// `user_data_dir`, and connects through the extension: the whole page
    /// API, with the profile as `contexts()[0]`. Closing the browser ends it.
    /// Branded Google Chrome 137+ no longer loads unpacked extensions from
    /// the command line; Edge, Chromium and Chrome for Testing do.
    pub async fn launch_with_extension(options: ExtensionLaunchOptions) -> Result<Browser> {
        let s = Self::services();
        let program = match options.executable_path {
            Some(p) => p,
            None => {
                let found = application::discovery::discover(&*s.fs, &*s.env, None)?;
                if domain::launch::BrowserKind::from_executable(&found.executable)
                    == domain::launch::BrowserKind::HeadlessShell
                {
                    return Err(Error::Launch(format!(
                        "{} cannot load extensions; pass executable_path (Chrome, Edge or Chromium)",
                        found.executable.display()
                    )));
                }
                found.executable
            }
        };
        let relay = Self::extension_relay(ExtensionRelayOptions {
            timeout: options.timeout,
            ..Default::default()
        })
        .await?;
        let args = domain::launch::extension_command_line(&domain::launch::ExtensionLaunchSpec {
            kind: domain::launch::BrowserKind::from_executable(&program),
            headless: options.headless,
            user_data_dir: options.user_data_dir,
            extension: options.extension_path,
            args: options.args,
            urls: vec![relay.connect_url()],
        });
        let process = s.launcher.spawn(program, args, Vec::new()).await?;
        match relay.wait().await {
            Ok(browser) => {
                browser.adopt_process(process);
                Ok(browser)
            }
            Err(e) => {
                process.shutdown().await;
                Err(e)
            }
        }
    }

    /// Drives the browser you already use, through the extension: opens the
    /// relay's page in the default browser (which must have the extension)
    /// and waits for it. Chrome and Edge give the whole page API; the
    /// browser's own profile is `contexts()[0]`.
    pub async fn connect_extension(options: ExtensionRelayOptions) -> Result<Browser> {
        let relay = Self::extension_relay(options).await?;
        adapters::extension_relay::open_in_default_browser(&relay.connect_url())?;
        relay.wait().await
    }

    /// Connects to a running browser's DevTools WebSocket URL.
    pub async fn connect_over_cdp(ws_endpoint: &str) -> Result<Browser> {
        let transport = adapters::transport::websocket::connect(ws_endpoint).await?;
        let cdp = Arc::new(adapters::cdp::Connection::spawn(transport));
        Browser::connect(Self::services(), cdp, Some(ws_endpoint.to_owned())).await
    }
}
