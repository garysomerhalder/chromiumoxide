//! Playwright's `_android`: devices through the adb server, Chrome and
//! WebViews on them as browser contexts (CDP over `localabstract:` sockets).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;

use crate::application::browser::Browser;
use crate::application::context::BrowserContext;
use crate::application::page::Page;
use crate::application::services::Services;
use crate::domain::android::{
    chrome_command_line, package_of, parse_devices, parse_webview_sockets,
};
use crate::domain::emulation::ContextOptions;
use crate::domain::error::{Error, Result};
use crate::ports::adb::{AdbAddress, AdbSocket};

const COMMAND_LINE_FILE: &str = "/data/local/tmp/chrome-command-line";

/// Playwright's `android.devices(options)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AndroidDevicesOptions {
    /// The adb server's host (default `127.0.0.1`).
    pub host: Option<String>,
    /// The adb server's port (default 5037).
    pub port: Option<u16>,
    /// Playwright's `omitDriverInstall`. The UI-automator driver is not
    /// used here, so it changes nothing.
    pub omit_driver_install: bool,
}

/// Playwright's `androidDevice.launchBrowser(options)`.
#[derive(Debug, Clone)]
pub struct AndroidLaunchOptions {
    /// The browser package (default `com.android.chrome`).
    pub pkg: String,
    /// Extra Chrome switches.
    pub args: Vec<String>,
    /// Options of the browser's context (never a default viewport).
    pub context: ContextOptions,
    /// How long to wait for the browser's DevTools socket (default 30 s).
    pub timeout: Duration,
}

impl Default for AndroidLaunchOptions {
    fn default() -> Self {
        Self {
            pkg: "com.android.chrome".into(),
            args: Vec::new(),
            context: ContextOptions::default(),
            timeout: Duration::from_secs(30),
        }
    }
}

/// Something that happened to a device.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum AndroidDeviceEvent {
    /// A WebView appeared (`androidDevice.on('webview')`).
    WebView(AndroidWebView),
    /// The device was closed (`androidDevice.on('close')`).
    Close,
}

struct DeviceInner {
    services: Services,
    server: AdbAddress,
    serial: String,
    model: String,
    closed: AtomicBool,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<AndroidDeviceEvent>>>,
    /// WebView socket names already reported.
    known_web_views: Mutex<Vec<String>>,
    /// Connections opened by `launch_browser` / `webView.page()`.
    browsers: Mutex<Vec<Browser>>,
}

impl DeviceInner {
    fn emit(&self, event: AndroidDeviceEvent) {
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.unbounded_send(event.clone()).is_ok());
    }
}

/// Playwright's `AndroidDevice`. Cheap to clone.
#[derive(Clone)]
pub struct AndroidDevice {
    inner: Arc<DeviceInner>,
}

impl std::fmt::Debug for AndroidDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AndroidDevice")
            .field("serial", &self.inner.serial)
            .field("model", &self.inner.model)
            .finish()
    }
}

/// `android.devices(options)`: the connected devices (state `device`).
pub async fn devices(
    services: Services,
    options: AndroidDevicesOptions,
) -> Result<Vec<AndroidDevice>> {
    let server = AdbAddress {
        host: options.host.unwrap_or_else(|| "127.0.0.1".into()),
        port: options.port.unwrap_or(5037),
    };
    let list = services.adb.devices(&server).await?;
    let mut out = Vec::new();
    for (serial, state) in parse_devices(&list) {
        if state != "device" {
            continue;
        }
        let model = services
            .adb
            .run(&server, &serial, "shell:getprop ro.product.model", None)
            .await?;
        let device = AndroidDevice {
            inner: Arc::new(DeviceInner {
                services: services.clone(),
                server: server.clone(),
                serial,
                model: String::from_utf8_lossy(&model).trim().to_owned(),
                closed: AtomicBool::new(false),
                subscribers: Mutex::new(Vec::new()),
                known_web_views: Mutex::new(Vec::new()),
                browsers: Mutex::new(Vec::new()),
            }),
        };
        device.poll_web_views();
        out.push(device);
    }
    Ok(out)
}

/// A unique DevTools socket name for `launchBrowser` (Playwright's
/// `playwright_<guid>_devtools_remote`).
fn socket_name(services: &Services) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let t = services.clock.wall_ms().to_bits();
    format!("playwright_{t:x}{n:x}_devtools_remote")
}

impl AndroidDevice {
    /// `androidDevice.serial()`.
    pub fn serial(&self) -> &str {
        &self.inner.serial
    }

    /// `androidDevice.model()`.
    pub fn model(&self) -> &str {
        &self.inner.model
    }

    /// `androidDevice.on(...)`: this device's events from now on.
    pub fn events(&self) -> impl futures::Stream<Item = AndroidDeviceEvent> + Send + use<> {
        let (tx, rx) = mpsc::unbounded();
        self.inner.subscribers.lock().unwrap().push(tx);
        rx
    }

    fn check_open(&self) -> Result<()> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::TargetClosed("Device is closed".into()));
        }
        Ok(())
    }

    async fn run(&self, command: &str, input: Option<Vec<u8>>) -> Result<Vec<u8>> {
        self.check_open()?;
        Ok(self
            .inner
            .services
            .adb
            .run(&self.inner.server, &self.inner.serial, command, input)
            .await?)
    }

    /// `androidDevice.shell(command)`: the command's output.
    pub async fn shell(&self, command: &str) -> Result<Vec<u8>> {
        self.run(&format!("shell:{command}"), None).await
    }

    /// `androidDevice.open(command)`: a raw socket to a device service,
    /// e.g. `shell:cat` or `localabstract:<name>`.
    pub async fn open(&self, command: &str) -> Result<AndroidSocket> {
        self.check_open()?;
        let socket = self
            .inner
            .services
            .adb
            .open(&self.inner.server, &self.inner.serial, command)
            .await?;
        Ok(AndroidSocket {
            socket: Arc::from(socket),
        })
    }

    /// `androidDevice.push(file, path, { mode })`.
    pub async fn push(&self, data: &[u8], path: &str, mode: u32) -> Result<()> {
        self.check_open()?;
        Ok(self
            .inner
            .services
            .adb
            .push(
                &self.inner.server,
                &self.inner.serial,
                data.to_vec(),
                path,
                mode,
            )
            .await?)
    }

    /// `androidDevice.installApk(file, { args })` (default args `-r -t -S`).
    pub async fn install_apk(&self, apk: &[u8], args: Option<&[&str]>) -> Result<()> {
        let args = args.unwrap_or(&["-r", "-t", "-S"]).join(" ");
        let out = self
            .run(
                &format!("shell:cmd package install {args} {}", apk.len()),
                Some(apk.to_vec()),
            )
            .await?;
        let out = String::from_utf8_lossy(&out);
        if out.contains("Failure") {
            return Err(Error::InvalidArgument(format!(
                "installApk: {}",
                out.trim()
            )));
        }
        Ok(())
    }

    /// `androidDevice.screenshot()`: a PNG of the screen.
    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        self.shell("screencap -p").await
    }

    /// `androidDevice.launchBrowser(options)`: starts the browser with
    /// Playwright's command line and returns its context.
    pub async fn launch_browser(&self, options: AndroidLaunchOptions) -> Result<BrowserContext> {
        let pkg = options.pkg.clone();
        self.shell(&format!("am force-stop {pkg}")).await?;
        let socket = socket_name(&self.inner.services);
        let line = chrome_command_line(&options.args, &socket);
        let encoded = base64::engine::general_purpose::STANDARD.encode(line);
        self.shell(&format!(
            "echo \"{encoded}\" | base64 -d > {COMMAND_LINE_FILE}"
        ))
        .await?;
        self.shell(&format!(
            "am start -a android.intent.action.VIEW -d about:blank {pkg}"
        ))
        .await?;
        let context = self
            .connect(&socket, options.context, options.timeout)
            .await?;
        if let Err(e) = self.shell(&format!("rm {COMMAND_LINE_FILE}")).await {
            let _ = context.close().await;
            return Err(e);
        }
        Ok(context)
    }

    /// Connects to a DevTools socket (polled until it accepts) and returns
    /// the browser's default context; closing it closes the connection.
    async fn connect(
        &self,
        socket: &str,
        context: ContextOptions,
        timeout: Duration,
    ) -> Result<BrowserContext> {
        let services = &self.inner.services;
        let attempt = async {
            loop {
                self.check_open()?;
                match services
                    .adb
                    .devtools(&self.inner.server, &self.inner.serial, socket)
                    .await
                {
                    Ok(cdp) => return Ok(cdp),
                    Err(_) => services.clock.sleep(Duration::from_millis(250)).await,
                }
            }
        };
        let cdp = crate::application::wait::with_timeout(
            &*services.clock,
            timeout,
            &format!("waiting for localabstract:{socket}"),
            attempt,
        )
        .await?;
        let context = ContextOptions {
            viewport: None,
            ..context
        };
        let (browser, ctx) =
            Browser::with_default_context(services.clone(), cdp, None, context).await?;
        *ctx.inner.owner.lock().unwrap() = Some(browser.clone());
        self.inner.browsers.lock().unwrap().push(browser);
        Ok(ctx)
    }

    /// `androidDevice.webViews()`: the WebViews with a DevTools socket.
    pub async fn web_views(&self) -> Result<Vec<AndroidWebView>> {
        let unix = self
            .shell("cat /proc/net/unix | grep webview_devtools_remote")
            .await?;
        let mut views = Vec::new();
        for (socket_name, pid) in parse_webview_sockets(&String::from_utf8_lossy(&unix)) {
            let pkg = if pid == -1 {
                String::new()
            } else {
                let ps = self.shell(&format!("ps -A | grep {pid}")).await?;
                package_of(&String::from_utf8_lossy(&ps), pid)
            };
            views.push(AndroidWebView {
                device: self.clone(),
                pid,
                pkg,
                socket_name,
            });
        }
        Ok(views)
    }

    /// `androidDevice.webView({ pkg }, { timeout })`: waits for a WebView of
    /// the package.
    pub async fn web_view(&self, pkg: &str, timeout: Duration) -> Result<AndroidWebView> {
        let clock = self.inner.services.clock.clone();
        let wait = async {
            loop {
                if let Some(v) = self.web_views().await?.into_iter().find(|v| v.pkg == pkg) {
                    return Ok(v);
                }
                clock.sleep(Duration::from_millis(500)).await;
            }
        };
        crate::application::wait::with_timeout(
            &*self.inner.services.clock,
            timeout,
            &format!("androidDevice.webView({pkg})"),
            wait,
        )
        .await
    }

    /// Reports new WebViews every 500 ms, like Playwright.
    fn poll_web_views(&self) {
        let weak = Arc::downgrade(&self.inner);
        let clock = self.inner.services.clock.clone();
        tokio::spawn(async move {
            loop {
                clock.sleep(Duration::from_millis(500)).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                if inner.closed.load(Ordering::SeqCst) {
                    return;
                }
                if inner.subscribers.lock().unwrap().is_empty() {
                    continue;
                }
                let device = AndroidDevice { inner };
                let Ok(views) = device.web_views().await else {
                    continue;
                };
                for v in views {
                    let new = {
                        let mut known = device.inner.known_web_views.lock().unwrap();
                        if known.contains(&v.socket_name) {
                            false
                        } else {
                            known.push(v.socket_name.clone());
                            true
                        }
                    };
                    if new {
                        device.inner.emit(AndroidDeviceEvent::WebView(v));
                    }
                }
            }
        });
    }

    /// `androidDevice.close()`: closes the browser connections opened here.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let browsers = std::mem::take(&mut *self.inner.browsers.lock().unwrap());
        for b in browsers {
            let _ = b.close().await;
        }
        self.inner.emit(AndroidDeviceEvent::Close);
        Ok(())
    }
}

/// Playwright's `AndroidWebView`.
#[derive(Clone, Debug)]
pub struct AndroidWebView {
    device: AndroidDevice,
    pid: i64,
    pkg: String,
    socket_name: String,
}

impl AndroidWebView {
    /// `webView.pid()` (`-1` if the socket name has none).
    pub fn pid(&self) -> i64 {
        self.pid
    }

    /// `webView.pkg()`.
    pub fn pkg(&self) -> &str {
        &self.pkg
    }

    /// The DevTools socket name.
    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }

    /// `webView.page()`: the WebView's page.
    pub async fn page(&self) -> Result<Page> {
        let timeout = Duration::from_secs(30);
        let context = self
            .device
            .connect(&self.socket_name, ContextOptions::default(), timeout)
            .await?;
        let mut events = context.events();
        if let Some(p) = context.pages().into_iter().next() {
            return Ok(p);
        }
        let wait = async {
            while let Some(e) = events.next().await {
                if let crate::application::context::ContextEvent::Page(p) = e {
                    return Ok(p);
                }
            }
            Err(Error::TargetClosed("WebView has been closed".into()))
        };
        crate::application::wait::with_timeout(
            &*self.device.inner.services.clock,
            timeout,
            "webView.page",
            wait,
        )
        .await
    }
}

/// Playwright's `AndroidSocket`: a raw connection to a device service.
#[derive(Clone)]
pub struct AndroidSocket {
    socket: Arc<dyn AdbSocket>,
}

impl std::fmt::Debug for AndroidSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AndroidSocket")
    }
}

impl AndroidSocket {
    /// `socket.write(data)`.
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        Ok(self.socket.write(data.to_vec()).await?)
    }

    /// `socket.on('data')`, ending at `close`. Can be taken once.
    pub fn data(&self) -> BoxStream<'static, Vec<u8>> {
        self.socket.data()
    }

    /// `socket.close()`.
    pub async fn close(&self) -> Result<()> {
        self.socket.close();
        Ok(())
    }
}
