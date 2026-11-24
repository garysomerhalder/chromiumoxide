//! Tauri apps (`chromiumoxide::Tauri`). Tauri has no browser of its own: on
//! Windows its windows are WebView2 (Chromium), reached over CDP, so each
//! window is a [`Page`] with the whole Chromium API.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;

use crate::application::browser::Browser;
use crate::application::context::{BrowserContext, ContextEvent};
use crate::application::page::Page;
use crate::application::services::Services;
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};
use crate::ports::TauriCommand;

/// Options for [`crate::Tauri::launch`].
#[derive(Debug, Clone)]
pub struct TauriLaunchOptions {
    /// The app's executable.
    pub executable_path: PathBuf,
    pub args: Vec<String>,
    /// Extra environment variables for the app.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the app's web view (default 60 s).
    pub timeout: Duration,
}

impl Default for TauriLaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: PathBuf::new(),
            args: Vec::new(),
            env: Vec::new(),
            timeout: Duration::from_secs(60),
        }
    }
}

/// A running Tauri app. Cheap to clone.
#[derive(Clone)]
pub struct TauriApp {
    inner: Arc<Inner>,
}

struct Inner {
    browser: Browser,
    context: BrowserContext,
    services: Services,
    pid: Option<u32>,
}

impl std::fmt::Debug for TauriApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TauriApp")
            .field("pid", &self.inner.pid)
            .finish()
    }
}

impl TauriApp {
    /// Starts the app and connects to its web view.
    pub async fn launch(services: Services, options: TauriLaunchOptions) -> Result<TauriApp> {
        let launched = services
            .launcher
            .launch_tauri(TauriCommand {
                program: options.executable_path.clone(),
                args: options.args.clone(),
                env: options.env.clone(),
                timeout_ms: options.timeout.as_millis() as u64,
            })
            .await?;
        let pid = launched.process.pid();
        let (browser, context) = Browser::with_default_context(
            services.clone(),
            launched.browser,
            Some(launched.process),
            ContextOptions {
                viewport: None,
                ..Default::default()
            },
        )
        .await?;
        Ok(TauriApp {
            inner: Arc::new(Inner {
                browser,
                context,
                services,
                pid,
            }),
        })
    }

    /// The app's process id.
    pub fn pid(&self) -> Option<u32> {
        self.inner.pid
    }

    /// The context of the app's windows.
    pub fn context(&self) -> BrowserContext {
        self.inner.context.clone()
    }

    /// The open windows.
    pub fn windows(&self) -> Vec<Page> {
        self.inner
            .context
            .pages()
            .into_iter()
            .filter(|p| !p.is_closed())
            .collect()
    }

    /// The first window, once it is open (30 s at most).
    pub async fn first_window(&self) -> Result<Page> {
        let mut events = self.inner.context.events();
        if let Some(p) = self.windows().into_iter().next() {
            return Ok(p);
        }
        let wait = async {
            while let Some(e) = events.next().await {
                if let ContextEvent::Page(p) = e {
                    return Ok(p);
                }
            }
            Err(Error::TargetClosed("the Tauri app closed".into()))
        };
        crate::application::wait::with_timeout(
            &*self.inner.services.clock,
            Duration::from_secs(30),
            "waiting for the Tauri app's first window",
            wait,
        )
        .await
    }

    /// Ends the app: its web view first, then the process (killed when it
    /// has not gone away by itself).
    pub async fn close(&self) -> Result<()> {
        let process = self.inner.browser.take_process();
        let _ = self.inner.browser.close().await;
        if let Some(process) = process {
            process.shutdown().await;
        }
        Ok(())
    }
}
