//! The set of ports the application runs on.

use std::sync::Arc;

use crate::ports::{BrowserLauncher, Clock, Environment, FileSystem, VideoEncoder};

/// Port implementations, injected by the composition root (or by tests).
#[derive(Clone)]
pub struct Services {
    pub fs: Arc<dyn FileSystem>,
    pub env: Arc<dyn Environment>,
    pub launcher: Arc<dyn BrowserLauncher>,
    pub clock: Arc<dyn Clock>,
    /// Encodes recorded videos (`recordVideo`).
    pub video: Arc<dyn VideoEncoder>,
    /// Sends `APIRequestContext` requests.
    pub http: Arc<dyn crate::ports::HttpClient>,
    /// Serves a browser connection to other clients (`browser.bind()`).
    pub bridge: Arc<dyn crate::ports::CdpBridge>,
    /// Serves a Firefox BiDi connection to other clients (`browser.bind()`).
    pub bidi_bridge: Arc<dyn crate::ports::BidiBridge>,
    /// Process signals (`handle_sigint` and friends).
    pub signals: Arc<dyn crate::ports::Signals>,
    /// The adb server (Android devices).
    pub adb: Arc<dyn crate::ports::adb::AdbClient>,
}
