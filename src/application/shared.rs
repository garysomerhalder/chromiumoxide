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
    /// Contexts created through this connection, by `browserContextId`.
    pub(crate) contexts: Mutex<HashMap<String, Arc<ContextInner>>>,
}
