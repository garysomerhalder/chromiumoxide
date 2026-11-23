//! Browser process port.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;

use crate::domain::error::Result;
use crate::ports::CdpSession;

/// How to reach the browser's DevTools endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    /// `--remote-debugging-pipe` (Playwright's default).
    #[default]
    Pipe,
    /// `--remote-debugging-port=0` and a WebSocket.
    WebSocket,
}

/// A fully resolved browser launch.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub channel: Channel,
    pub user_data_dir: PathBuf,
    /// How long to wait for the browser to become reachable.
    pub timeout_ms: u64,
}

/// A running browser process.
pub trait BrowserProcess: Send + Sync {
    fn pid(&self) -> Option<u32>;
    /// Waits for the process to exit by itself for a grace period, then kills it.
    fn shutdown(&self) -> BoxFuture<'static, ()>;
}

/// A launched, connected browser.
pub struct Launched {
    pub cdp: Arc<dyn CdpSession>,
    pub process: Box<dyn BrowserProcess>,
    /// The DevTools WebSocket URL, when launched with [`Channel::WebSocket`].
    pub ws_endpoint: Option<String>,
}

/// Starts browser processes.
pub trait BrowserLauncher: Send + Sync {
    fn launch(&self, command: LaunchCommand) -> BoxFuture<'static, Result<Launched>>;
    /// Starts an Electron app and connects to its renderer (CDP) and main
    /// process (Node inspector) endpoints, printed on its stderr.
    fn launch_electron(
        &self,
        command: ElectronCommand,
    ) -> BoxFuture<'static, Result<LaunchedElectron>> {
        let _ = command;
        Box::pin(async {
            Err(crate::domain::error::Error::Launch(
                "this launcher cannot start Electron apps".into(),
            ))
        })
    }
}

/// An Electron launch (Playwright's `electron.launch`).
#[derive(Debug, Clone, PartialEq)]
pub struct ElectronCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The whole environment (`None`: inherit this process's), without
    /// `NODE_OPTIONS` either way.
    pub env: Option<Vec<(String, String)>>,
    pub cwd: Option<PathBuf>,
    pub timeout_ms: u64,
}

/// A launched, connected Electron app.
pub struct LaunchedElectron {
    /// The DevTools endpoint (windows are its page targets).
    pub browser: Arc<dyn CdpSession>,
    /// The main process's Node inspector.
    pub node: Arc<dyn CdpSession>,
    pub process: Box<dyn BrowserProcess>,
}
