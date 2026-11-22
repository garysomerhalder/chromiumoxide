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
}
