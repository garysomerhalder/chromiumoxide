//! Ports: the traits the application needs from the outside world.

pub mod bridge;
pub mod cdp;
pub mod clock;
pub mod env;
pub mod fs;
pub mod http;
pub mod process;
pub mod transport;
pub mod video;

pub use bridge::{BoundBridge, CdpBridge};
pub use cdp::{CdpSession, EventStream, RawEvent};
pub use clock::Clock;
pub use env::Environment;
pub use fs::FileSystem;
pub use http::HttpClient;
pub use process::{BrowserLauncher, BrowserProcess, Channel, LaunchCommand, Launched};
pub use transport::Transport;
pub use video::{VideoEncoder, VideoSink};
