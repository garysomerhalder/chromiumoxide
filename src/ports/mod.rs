//! Ports: the traits the application needs from the outside world.

pub mod cdp;
pub mod env;
pub mod fs;
pub mod transport;

pub use cdp::{CdpSession, EventStream, RawEvent};
pub use env::Environment;
pub use fs::FileSystem;
pub use transport::Transport;
