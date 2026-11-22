//! Use cases: launch a browser, query and close it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::application::discovery::discover;
use crate::application::services::Services;
use crate::domain::error::Result;
use crate::domain::launch::{LaunchSpec, command_line};
use crate::ports::{BrowserProcess, CdpSession, Channel, LaunchCommand};

/// Playwright's `browserType.launch(options)`.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchOptions {
    /// Use this executable instead of discovering one.
    pub executable_path: Option<PathBuf>,
    /// Headless (default `true`).
    pub headless: bool,
    /// Extra command-line switches.
    pub args: Vec<String>,
    /// Drop the default argument preset.
    pub ignore_default_args: bool,
    /// Extra environment variables for the browser process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the browser to start (default 180 s, like Playwright).
    pub timeout: Duration,
    /// Pipe (default) or WebSocket.
    pub channel: Channel,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            executable_path: None,
            headless: true,
            args: Vec::new(),
            ignore_default_args: false,
            env: Vec::new(),
            timeout: Duration::from_secs(180),
            channel: Channel::Pipe,
        }
    }
}

/// A connected browser.
#[derive(Clone)]
pub struct Browser {
    inner: Arc<BrowserInner>,
}

struct BrowserInner {
    cdp: Arc<dyn CdpSession>,
    services: Services,
    process: Mutex<Option<Box<dyn BrowserProcess>>>,
    user_data_dir: Option<PathBuf>,
    executable: Option<PathBuf>,
    closed: AtomicBool,
}

impl Browser {
    /// Discovers, launches and connects to a browser.
    pub async fn launch(services: Services, options: LaunchOptions) -> Result<Browser> {
        let found = discover(
            &*services.fs,
            &*services.env,
            options.executable_path.as_deref(),
        )?;
        let user_data_dir = services.fs.create_temp_dir("chromiumoxide-profile-")?;
        let spec = LaunchSpec {
            kind: found.kind,
            headless: options.headless,
            args: options.args.clone(),
            ignore_default_args: options.ignore_default_args,
            user_data_dir: user_data_dir.clone(),
        };
        let mut args = command_line(&spec);
        if options.channel == Channel::WebSocket {
            for a in &mut args {
                if a == "--remote-debugging-pipe" {
                    *a = "--remote-debugging-port=0".to_owned();
                }
            }
        }
        let command = LaunchCommand {
            program: found.executable.clone(),
            args,
            env: options.env.clone(),
            channel: options.channel,
            user_data_dir: user_data_dir.clone(),
            timeout_ms: options.timeout.as_millis() as u64,
        };
        let launched = match services.launcher.launch(command).await {
            Ok(l) => l,
            Err(e) => {
                let _ = services.fs.remove_dir_all(&user_data_dir);
                return Err(e);
            }
        };
        let browser = Browser {
            inner: Arc::new(BrowserInner {
                cdp: launched.cdp,
                services,
                process: Mutex::new(Some(launched.process)),
                user_data_dir: Some(user_data_dir),
                executable: Some(found.executable),
                closed: AtomicBool::new(false),
            }),
        };
        if let Err(e) = browser.handshake().await {
            let _ = browser.close().await;
            return Err(e);
        }
        Ok(browser)
    }

    /// Playwright's connection handshake on the browser target.
    async fn handshake(&self) -> Result<()> {
        let cdp = &self.inner.cdp;
        cdp.call(None, "Browser.getVersion", json!({})).await?;
        cdp.call(
            None,
            "Target.setAutoAttach",
            json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}),
        )
        .await?;
        Ok(())
    }

    /// The browser's product string, e.g. `HeadlessChrome/157.0.8103.78`.
    pub async fn version(&self) -> Result<String> {
        let v = self
            .inner
            .cdp
            .call(None, "Browser.getVersion", json!({}))
            .await?;
        Ok(v.get("product")
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_owned())
    }

    /// The temporary profile directory of this launch.
    pub fn user_data_dir(&self) -> Option<PathBuf> {
        self.inner.user_data_dir.clone()
    }

    /// The executable that was launched.
    pub fn executable_path(&self) -> Option<PathBuf> {
        self.inner.executable.clone()
    }

    pub fn is_connected(&self) -> bool {
        !self.inner.closed.load(Ordering::SeqCst)
    }

    /// Raw protocol access to the browser target.
    pub fn cdp(&self) -> Arc<dyn CdpSession> {
        self.inner.cdp.clone()
    }

    /// Closes the browser, ends the process and deletes the temporary profile.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let _ = self.inner.cdp.call(None, "Browser.close", json!({})).await;
        let process = self.inner.process.lock().unwrap().take();
        if let Some(process) = process {
            process.shutdown().await;
        }
        if let Some(dir) = &self.inner.user_data_dir {
            let _ = self.inner.services.fs.remove_dir_all(dir);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::FileSystem;
    use crate::testing::clock::TestClock;
    use crate::testing::memory::{MapEnvironment, MemoryFileSystem};
    use crate::testing::process::FakeLauncher;

    const SHELL: &str = "/opt/pw-browsers/chromium_headless_shell-1243/chrome-headless-shell-linux64/chrome-headless-shell";

    fn services(launcher: &FakeLauncher) -> (Services, Arc<MemoryFileSystem>) {
        let fs = Arc::new(MemoryFileSystem::with_files([SHELL]));
        let s = Services {
            fs: fs.clone(),
            env: Arc::new(MapEnvironment::linux("/home/u")),
            launcher: Arc::new(launcher.clone()),
            clock: Arc::new(TestClock),
        };
        (s, fs)
    }

    #[tokio::test]
    async fn launch_runs_the_discovered_browser_with_the_preset_and_a_fresh_profile() {
        let launcher = FakeLauncher::default();
        let (s, fs) = services(&launcher);
        let browser = Browser::launch(
            s,
            LaunchOptions {
                args: vec!["--lang=de".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let cmd = launcher.commands.lock().unwrap()[0].clone();
        assert_eq!(cmd.program, PathBuf::from(SHELL));
        let dir = browser.user_data_dir().unwrap();
        assert!(dir.to_string_lossy().contains("chromiumoxide-profile-"));
        assert!(fs.is_dir(&dir));
        assert_eq!(cmd.user_data_dir, dir);
        assert!(cmd.args.contains(&"--headless".to_owned()));
        assert!(cmd.args.contains(&"--lang=de".to_owned()));
        assert!(
            cmd.args
                .contains(&format!("--user-data-dir={}", dir.display()))
        );
        assert_eq!(cmd.channel, Channel::Pipe);
        assert_eq!(browser.executable_path(), Some(PathBuf::from(SHELL)));
    }

    #[tokio::test]
    async fn every_launch_gets_its_own_profile() {
        let launcher = FakeLauncher::default();
        let (s, _) = services(&launcher);
        let a = Browser::launch(s.clone(), LaunchOptions::default())
            .await
            .unwrap();
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        assert_ne!(a.user_data_dir(), b.user_data_dir());
    }

    #[tokio::test]
    async fn connect_handshake_matches_playwright() {
        let launcher = FakeLauncher::default();
        let (s, _) = services(&launcher);
        let _b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        let calls = launcher.cdp.calls();
        assert_eq!(calls[0].method, "Browser.getVersion");
        assert_eq!(calls[1].method, "Target.setAutoAttach");
        assert_eq!(
            calls[1].params,
            json!({"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true})
        );
        assert_eq!(calls[1].session_id, None);
    }

    #[tokio::test]
    async fn version_reports_the_product() {
        let launcher = FakeLauncher::default();
        launcher.cdp.on_call(|_, c| {
            Ok(if c.method == "Browser.getVersion" {
                json!({"product": "HeadlessChrome/157.0"})
            } else {
                json!({})
            })
        });
        let (s, _) = services(&launcher);
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        assert_eq!(b.version().await.unwrap(), "HeadlessChrome/157.0");
    }

    #[tokio::test]
    async fn close_sends_browser_close_stops_the_process_and_removes_the_profile() {
        let launcher = FakeLauncher::default();
        let (s, fs) = services(&launcher);
        let b = Browser::launch(s, LaunchOptions::default()).await.unwrap();
        let dir = b.user_data_dir().unwrap();
        b.close().await.unwrap();
        assert!(launcher.cdp.methods().contains(&"Browser.close".to_owned()));
        assert!(launcher.shut_down.load(Ordering::SeqCst));
        assert!(!fs.is_dir(&dir));
        assert!(!b.is_connected());
        b.close().await.unwrap(); // idempotent
    }

    #[tokio::test]
    async fn headful_and_full_chrome_use_new_headless_rules() {
        let launcher = FakeLauncher::default();
        let (mut s, _) = services(&launcher);
        s.fs = Arc::new(MemoryFileSystem::with_files(["/usr/bin/google-chrome"]));
        let opts = LaunchOptions {
            executable_path: Some("/usr/bin/google-chrome".into()),
            ..Default::default()
        };
        Browser::launch(s, opts).await.unwrap();
        let cmd = launcher.commands.lock().unwrap()[0].clone();
        assert!(cmd.args.contains(&"--headless=new".to_owned()));
    }
}
