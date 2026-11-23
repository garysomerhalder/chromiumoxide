//! Ports: the traits the application needs from the outside world.

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

pub use bridge::{BoundBridge, BridgeAddress, CdpBridge};
pub use cdp::{CdpSession, EventStream, RawEvent};
pub use clock::Clock;
pub use env::Environment;
pub use fs::FileSystem;
pub use http::HttpClient;
pub use process::{
    BrowserLauncher, BrowserProcess, Channel, ElectronCommand, LaunchCommand, Launched,
    LaunchedElectron,
};
pub use signals::{Signal, Signals};
pub use transport::Transport;
pub use video::{VideoEncoder, VideoSink};
