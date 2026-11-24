//! Ports: the traits the application needs from the outside world.

pub mod adb;
pub mod bidi;
pub mod bridge;
pub mod cdp;
pub mod clock;
pub mod env;
pub mod fs;
pub mod http;
pub mod process;
pub mod signals;
pub mod transport;
pub mod video;
pub mod webkit;

pub use bidi::BidiSession;
pub use bridge::{BidiBridge, BoundBridge, BridgeAddress, CdpBridge};
pub use cdp::{CdpSession, EventStream, RawEvent};
pub use clock::Clock;
pub use env::Environment;
pub use fs::FileSystem;
pub use http::HttpClient;
pub use process::{
    BrowserLauncher, BrowserProcess, Channel, ElectronCommand, FirefoxCommand, LaunchCommand,
    Launched, LaunchedElectron, LaunchedFirefox, LaunchedTauri, LaunchedWebDriver, LaunchedWebKit,
    TauriCommand, TauriConnection, WebDriverCommand, WebKitCommand,
};
pub use signals::{Signal, Signals};
pub use transport::Transport;
pub use video::{VideoEncoder, VideoSink};
pub use webkit::WebKitSession;
