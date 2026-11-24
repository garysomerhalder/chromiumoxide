//! Safari (`chromiumoxide::Safari`): Apple's `safaridriver` (W3C WebDriver)
//! starts Safari in an automation window. Safari has no protocol like CDP or
//! BiDi for clients outside Apple, so this is the WebDriver page API, the
//! same as for Tauri on Linux. Any W3C WebDriver server works with
//! `driver_path` and `capabilities` (tests use WebKitGTK's
//! `WebKitWebDriver` on Linux).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::application::services::Services;
use crate::application::tauri::WebDriverWindow;
use crate::domain::error::{Error, Result};
use crate::domain::safari::{SAFARIDRIVER, new_session};
use crate::ports::{BrowserProcess, WebDriverCommand};

/// Options for [`crate::Safari::launch`].
#[derive(Debug, Clone)]
pub struct SafariLaunchOptions {
    /// The WebDriver server (default `/usr/bin/safaridriver`; enable it once
    /// with `safaridriver --enable`).
    pub driver_path: Option<PathBuf>,
    /// The session's `alwaysMatch` capabilities (default
    /// `{"browserName": "safari"}`).
    pub capabilities: Option<Value>,
    /// Extra environment variables for the driver.
    pub env: Vec<(String, String)>,
    /// Maximum time to start the driver and open the window (default 60 s).
    pub timeout: Duration,
}

impl Default for SafariLaunchOptions {
    fn default() -> Self {
        Self {
            driver_path: None,
            capabilities: None,
            env: Vec::new(),
            timeout: Duration::from_secs(60),
        }
    }
}

/// A Safari window over WebDriver.
pub struct SafariBrowser {
    page: WebDriverWindow,
    capabilities: Value,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
}

impl std::fmt::Debug for SafariBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafariBrowser")
            .field("page", &self.page)
            .finish()
    }
}

impl SafariBrowser {
    /// Starts the WebDriver server and opens a session.
    pub async fn launch(services: Services, options: SafariLaunchOptions) -> Result<Self> {
        let driver = options
            .driver_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(SAFARIDRIVER));
        if !services.fs.is_file(&driver) {
            return Err(Error::Launch(format!(
                "{} not found: Safari is automated through safaridriver, which comes with \
                 macOS (run `safaridriver --enable` once)",
                driver.display()
            )));
        }
        let launched = services
            .launcher
            .launch_webdriver(WebDriverCommand {
                driver,
                args: Vec::new(),
                env: options.env.clone(),
                new_session: new_session(options.capabilities.as_ref()),
                timeout_ms: options.timeout.as_millis() as u64,
                embedded: false,
            })
            .await?;
        Ok(SafariBrowser {
            page: WebDriverWindow::new(
                services,
                format!("{}/session/{}", launched.endpoint, launched.session),
            ),
            capabilities: launched.capabilities,
            process: Mutex::new(Some(launched.process)),
        })
    }

    /// The automation window's page.
    pub fn page(&self) -> &WebDriverWindow {
        &self.page
    }

    /// `browser.version()`: the browser version the driver reports.
    pub fn version(&self) -> String {
        self.capabilities["browserVersion"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// Ends the session (Safari closes its window) and the driver.
    pub async fn close(&self) -> Result<()> {
        let _ = self.page.end_session().await;
        let process = self.process.lock().unwrap().take();
        if let Some(process) = process {
            process.shutdown().await;
        }
        Ok(())
    }
}
