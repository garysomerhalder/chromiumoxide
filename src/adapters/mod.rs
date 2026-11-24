//! Adapters: implementations of the ports (CDP, transports, processes, OS).

pub mod adb;
pub mod bidi;
pub mod bidi_bridge;
pub mod cdp;
pub mod clock;
pub mod electron;
pub mod http;
pub mod os;
pub mod process;
pub mod signals;
pub mod transport;
pub mod video;
pub mod webkit;
#[cfg(windows)]
pub mod win_pipes;
