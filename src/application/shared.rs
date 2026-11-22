//! State shared by a browser, its contexts and pages.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::application::context::ContextInner;
use crate::application::services::Services;
use crate::domain::emulation::FontPlatform;
use crate::ports::CdpSession;

pub(crate) struct Shared {
    pub(crate) cdp: Arc<dyn CdpSession>,
    pub(crate) services: Services,
    pub(crate) headless: bool,
    pub(crate) font_platform: FontPlatform,
    /// e.g. `157.0.8103.78`.
    pub(crate) browser_version: String,
    /// Contexts created through this connection, by `browserContextId`.
    pub(crate) contexts: Mutex<HashMap<String, Arc<ContextInner>>>,
    /// Where downloads are saved (created on first use).
    pub(crate) downloads_dir: Mutex<Option<std::path::PathBuf>>,
    /// Downloads in progress, by guid.
    pub(crate) downloads: Mutex<HashMap<String, crate::application::download::Download>>,
}
