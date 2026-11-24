//! Launches the browser process and connects to it (pipe or WebSocket).

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::adapters::cdp::Connection;
use crate::domain::error::{Error, Result};
use crate::ports::{
    BrowserLauncher, BrowserProcess, Channel, ElectronCommand, FirefoxCommand, LaunchCommand,
    Launched, LaunchedElectron, LaunchedFirefox, LaunchedTauri, LaunchedWebKit, TauriCommand,
    TauriConnection, Transport, WebKitCommand,
};

/// Spawns browsers with tokio.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioLauncher;

impl BrowserLauncher for TokioLauncher {
    fn launch(&self, command: LaunchCommand) -> BoxFuture<'static, Result<Launched>> {
        launch(command).boxed()
    }

    fn launch_electron(
        &self,
        command: ElectronCommand,
    ) -> BoxFuture<'static, Result<LaunchedElectron>> {
        launch_electron(command).boxed()
    }

    fn launch_firefox(
        &self,
        command: FirefoxCommand,
    ) -> BoxFuture<'static, Result<LaunchedFirefox>> {
        launch_firefox(command).boxed()
    }

    fn launch_webkit(&self, command: WebKitCommand) -> BoxFuture<'static, Result<LaunchedWebKit>> {
        launch_webkit(command).boxed()
    }

    fn launch_tauri(&self, command: TauriCommand) -> BoxFuture<'static, Result<LaunchedTauri>> {
        launch_tauri(command).boxed()
    }
}

/// Windows: a Tauri app's WebView2 gets `--remote-debugging-port` through
/// `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`; its DevTools endpoint comes
/// from `/json/version` on that port.
#[cfg(windows)]
async fn launch_tauri(command: TauriCommand) -> Result<LaunchedTauri> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| Error::Launch(format!("cannot pick a port: {e}")))?
        .port();
    let mut extra = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
    if let Some((_, v)) = command
        .env
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"))
    {
        extra = v.clone();
    }
    let extra = format!("{extra} --remote-debugging-port={port}")
        .trim()
        .to_owned();
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .envs(command.env.iter().cloned())
        .env("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", extra)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", command.program.display())))?;
    let pid = child.id();
    let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
    let deadline = tokio::time::Instant::now() + Duration::from_millis(command.timeout_ms.max(1));
    let url = loop {
        if let Some(url) = devtools_browser_url(port).await {
            break url;
        }
        if let Some(c) = child.lock().await.as_mut()
            && let Ok(Some(status)) = c.try_wait()
        {
            return Err(Error::Launch(format!("the Tauri app exited ({status})")));
        }
        if tokio::time::Instant::now() > deadline {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.kill().await;
            }
            return Err(Error::Timeout {
                ms: command.timeout_ms,
                action: "waiting for the Tauri app's WebView2 DevTools".into(),
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let transport = crate::adapters::transport::websocket::connect(&url).await?;
    Ok(LaunchedTauri {
        connection: TauriConnection::Cdp(Arc::new(Connection::spawn(transport))),
        process: Box::new(Process { child, pid }),
    })
}

/// `webSocketDebuggerUrl` from `http://127.0.0.1:<port>/json/version`.
#[cfg(windows)]
async fn devtools_browser_url(port: u16) -> Option<String> {
    use tokio::io::AsyncWriteExt;
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    stream
        .write_all(b"GET /json/version HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .ok()?;
    let mut body = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut stream, &mut body)
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&body);
    let json = text.split_once("\r\n\r\n")?.1;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v["webSocketDebuggerUrl"].as_str().map(str::to_owned)
}

/// Linux: `WebKitWebDriver` (WebKitGTK's WebDriver server, as Tauri's own
/// `tauri-driver` uses) starts the app in a WebDriver session. Tauri apps
/// accept automation with `TAURI_WEBVIEW_AUTOMATION=true`.
#[cfg(not(windows))]
async fn launch_tauri(command: TauriCommand) -> Result<LaunchedTauri> {
    if cfg!(target_os = "macos") {
        return Err(Error::Launch(
            "Tauri apps cannot be automated on macOS: WKWebView has no WebDriver or DevTools \
             interface for embedding apps"
                .into(),
        ));
    }
    let driver = std::env::var_os("WEBKIT_WEBDRIVER")
        .map(std::path::PathBuf::from)
        .or_else(|| which::which("WebKitWebDriver").ok())
        .ok_or_else(|| {
            Error::Launch(
                "WebKitWebDriver not found (install webkit2gtk-driver, or set WEBKIT_WEBDRIVER)"
                    .into(),
            )
        })?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| Error::Launch(format!("cannot pick a port: {e}")))?
        .port();
    let mut cmd = Command::new(&driver);
    cmd.arg(format!("--port={port}"))
        .envs(command.env.iter().cloned())
        .env("TAURI_WEBVIEW_AUTOMATION", "true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", driver.display())))?;
    let pid = child.id();
    let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
    let process = Process { child, pid };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(command.timeout_ms.max(1));
    while http_json(port, "GET", "/status", None).await.is_none() {
        if tokio::time::Instant::now() > deadline {
            return Err(Error::Timeout {
                ms: command.timeout_ms,
                action: "waiting for WebKitWebDriver".into(),
            });
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let caps = serde_json::json!({"capabilities": {"alwaysMatch": {"webkitgtk:browserOptions": {
        "binary": command.program, "args": command.args}}}});
    let created = tokio::time::timeout(
        Duration::from_millis(command.timeout_ms.max(1)),
        http_json(port, "POST", "/session", Some(caps)),
    )
    .await
    .ok()
    .flatten()
    .ok_or_else(|| Error::Launch("WebKitWebDriver did not start the app".into()))?;
    let session = created
        .pointer("/value/sessionId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            Error::Launch(format!(
                "WebKitWebDriver could not start the app: {}",
                created
                    .pointer("/value/message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
            ))
        })?
        .to_owned();
    Ok(LaunchedTauri {
        connection: TauriConnection::WebDriver {
            endpoint: format!("http://127.0.0.1:{port}"),
            session,
        },
        process: Box::new(process),
    })
}

/// One HTTP/1.0 JSON exchange with a local server; `None` when it cannot
/// connect or the answer is not JSON.
#[cfg(not(windows))]
async fn http_json(
    port: u16,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .ok()?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.ok()?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.ok()?;
    let text = String::from_utf8_lossy(&answer);
    serde_json::from_str(text.split_once("\r\n\r\n")?.1).ok()
}

/// Starts WebKit with its inspector pipe on fds 3 and 4.
#[cfg(unix)]
async fn launch_webkit(command: WebKitCommand) -> Result<LaunchedWebKit> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .envs(command.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let pipes = unix_pipes::attach(&mut cmd)?;
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", command.program.display())))?;
    let pid = child.id();
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "chromiumoxide::webkit", "{line}");
            }
        });
    }
    let (reader, writer) = pipes.into_parent()?;
    let transport = crate::adapters::transport::pipe::pipe_transport(reader, writer);
    Ok(LaunchedWebKit {
        session: Arc::new(crate::adapters::webkit::WebKitConnection::spawn(transport)),
        process: Box::new(Process {
            child: Arc::new(tokio::sync::Mutex::new(Some(child))),
            pid,
        }),
    })
}

/// Windows: WebKit's inspector pipe on its runtime's fds 3 and 4.
#[cfg(windows)]
async fn launch_webkit(command: WebKitCommand) -> Result<LaunchedWebKit> {
    let (transport, process) = windows_pipe_transport(
        &command.program,
        &command.args,
        &command.env,
        "chromiumoxide::webkit",
    )?;
    Ok(LaunchedWebKit {
        session: Arc::new(crate::adapters::webkit::WebKitConnection::spawn(transport)),
        process,
    })
}

/// Windows: Chromium's `--remote-debugging-pipe` on fds 3 and 4, like
/// Playwright (Node passes them the same way).
#[cfg(windows)]
fn launch_windows_pipe(command: LaunchCommand) -> Result<Launched> {
    let (transport, process) = windows_pipe_transport(
        &command.program,
        &command.args,
        &command.env,
        "chromiumoxide::browser",
    )?;
    Ok(Launched {
        cdp: Arc::new(Connection::spawn(transport)),
        process,
        ws_endpoint: None,
    })
}

/// Starts `program` with pipes on its fds 3 and 4; the transport over them
/// and the process. Stderr goes to the log.
#[cfg(windows)]
fn windows_pipe_transport(
    program: &std::path::Path,
    args: &[String],
    env: &[(String, String)],
    log_target: &'static str,
) -> Result<(Transport, Box<dyn BrowserProcess>)> {
    let spawned = crate::adapters::win_pipes::spawn(program, args, env)?;
    let stderr = tokio::fs::File::from_std(spawned.stderr);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "chromiumoxide::browser", "[{log_target}] {line}");
        }
    });
    let transport = crate::adapters::transport::pipe::pipe_transport(
        tokio::fs::File::from_std(spawned.from_child),
        tokio::fs::File::from_std(spawned.to_child),
    );
    Ok((transport, Box::new(spawned.child)))
}

/// Firefox prints `WebDriver BiDi listening on ws://127.0.0.1:<port>` on
/// stderr; the BiDi session endpoint is `<that>/session`.
async fn launch_firefox(command: FirefoxCommand) -> Result<LaunchedFirefox> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .envs(command.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", command.program.display())))?;
    let pid = child.id();
    let stderr = child.stderr.take().expect("piped stderr");
    let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "chromiumoxide::firefox", "{line}");
            let _ = tx.send(line);
        }
    });
    let mut log = Vec::new();
    let found = tokio::time::timeout(Duration::from_millis(command.timeout_ms.max(1)), async {
        while let Some(line) = rx.recv().await {
            if let Some(url) = line.strip_prefix("WebDriver BiDi listening on ") {
                return Some(url.trim().to_owned());
            }
            log.push(line);
        }
        None
    })
    .await;
    let endpoint = match found {
        Ok(Some(url)) => url,
        other => {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.kill().await;
            }
            let reason = if other.is_err() {
                "Timed out waiting for Firefox's WebDriver BiDi endpoint"
            } else {
                "Firefox exited before it was ready"
            };
            let tail = log
                .iter()
                .rev()
                .take(20)
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(Error::Launch(format!("{reason}\n{tail}")));
        }
    };
    // Keep draining stderr so Firefox never blocks on a full pipe.
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let transport =
        crate::adapters::transport::websocket::connect(&format!("{endpoint}/session")).await?;
    Ok(LaunchedFirefox {
        bidi: Arc::new(crate::adapters::bidi::BidiConnection::spawn(transport)),
        process: Box::new(Process { child, pid }),
        ws_endpoint: endpoint,
    })
}

/// Playwright's `electron.launch`: the app prints `Debugger listening on
/// ws://…` (main process) and `DevTools listening on ws://…` (renderers) on
/// stderr; both are connected.
async fn launch_electron(command: ElectronCommand) -> Result<LaunchedElectron> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(env) = &command.env {
        cmd.env_clear().envs(env.iter().cloned());
    }
    cmd.env_remove("NODE_OPTIONS");
    if let Some(dir) = &command.cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", command.program.display())))?;
    let pid = child.id();
    let stderr = child.stderr.take().expect("piped stderr");
    let child = Arc::new(tokio::sync::Mutex::new(Some(child)));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(target: "chromiumoxide::electron", "{line}");
            let _ = tx.send(line);
        }
    });
    let mut node_url = None;
    let mut devtools_url = None;
    let mut log = Vec::new();
    let found = tokio::time::timeout(Duration::from_millis(command.timeout_ms.max(1)), async {
        while node_url.is_none() || devtools_url.is_none() {
            let Some(line) = rx.recv().await else {
                return false;
            };
            if let Some(url) = line.strip_prefix("Debugger listening on ") {
                node_url = Some(url.trim().to_owned());
            } else if let Some(url) = line.strip_prefix("DevTools listening on ") {
                devtools_url = Some(url.trim().to_owned());
            } else if line.contains("Unable to open X display") || line.contains("Missing X server")
            {
                log.push(line);
                return false;
            } else {
                log.push(line);
            }
        }
        true
    })
    .await;
    let fail = |msg: String, log: &[String]| {
        let tail = log
            .iter()
            .rev()
            .take(20)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        Error::Launch(format!("{msg}\n{tail}"))
    };
    let (node_url, devtools_url) = match (found, node_url, devtools_url) {
        (Ok(true), Some(n), Some(d)) => (n, d),
        (Err(_), ..) => {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.kill().await;
            }
            return Err(fail(
                "Timed out waiting for the Electron app's debugging endpoints".into(),
                &log,
            ));
        }
        _ => {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.kill().await;
            }
            let hint = if log
                .iter()
                .any(|l| l.contains("X display") || l.contains("X server"))
            {
                "Unable to open X display! Run under Xvfb (e.g. xvfb-run) or set DISPLAY."
            } else {
                "The Electron app exited before it was ready"
            };
            return Err(fail(hint.into(), &log));
        }
    };
    // Keep draining stderr so the app never blocks on a full pipe.
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let node = crate::adapters::transport::websocket::connect(&node_url).await?;
    let browser = crate::adapters::transport::websocket::connect(&devtools_url).await?;
    Ok(LaunchedElectron {
        browser: Arc::new(Connection::spawn(browser)),
        node: Arc::new(Connection::spawn(node)),
        process: Box::new(Process { child, pid }),
    })
}

struct Process {
    child: Arc<tokio::sync::Mutex<Option<Child>>>,
    pid: Option<u32>,
}

impl BrowserProcess for Process {
    fn pid(&self) -> Option<u32> {
        self.pid
    }

    fn shutdown(&self) -> BoxFuture<'static, ()> {
        let child = self.child.clone();
        async move {
            let Some(mut child) = child.lock().await.take() else {
                return;
            };
            if tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .is_err()
            {
                let _ = child.kill().await;
            }
        }
        .boxed()
    }
}

type StderrTail = Arc<Mutex<VecDeque<String>>>;

async fn launch(command: LaunchCommand) -> Result<Launched> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .envs(command.env.iter().cloned())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(unix)]
    let pipes = match command.channel {
        Channel::Pipe => Some(unix_pipes::attach(&mut cmd)?),
        Channel::WebSocket => None,
    };
    #[cfg(windows)]
    if command.channel == Channel::Pipe {
        return launch_windows_pipe(command);
    }

    if command.channel == Channel::WebSocket {
        // A reused profile still has the previous browser's port file.
        let _ = std::fs::remove_file(command.user_data_dir.join("DevToolsActivePort"));
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Launch(format!("{}: {e}", command.program.display())))?;
    let pid = child.id();
    let tail: StderrTail = Arc::default();
    if let Some(stderr) = child.stderr.take() {
        let tail = tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "chromiumoxide::browser", "{line}");
                let mut t = tail.lock().unwrap();
                if t.len() == 20 {
                    t.pop_front();
                }
                t.push_back(line);
            }
        });
    }
    let child = Arc::new(tokio::sync::Mutex::new(Some(child)));

    let connected: Result<(Transport, Option<String>)> = match command.channel {
        #[cfg(unix)]
        Channel::Pipe => {
            let (reader, writer) = pipes.expect("pipes").into_parent()?;
            Ok((
                crate::adapters::transport::pipe::pipe_transport(reader, writer),
                None,
            ))
        }
        #[cfg(not(unix))]
        Channel::Pipe => unreachable!("handled above"),
        Channel::WebSocket => {
            match wait_for_ws_endpoint(&command.user_data_dir, &child, command.timeout_ms).await {
                Ok(url) => crate::adapters::transport::websocket::connect(&url)
                    .await
                    .map(|t| (t, Some(url))),
                Err(e) => Err(e),
            }
        }
    };
    let (transport, ws_endpoint) = match connected {
        Ok(c) => c,
        Err(e) => {
            if let Some(mut c) = child.lock().await.take() {
                let _ = c.kill().await;
            }
            let log = tail
                .lock()
                .unwrap()
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            return Err(Error::Launch(format!("{e}\n{log}")));
        }
    };
    Ok(Launched {
        cdp: Arc::new(Connection::spawn(transport)),
        process: Box::new(Process { child, pid }),
        ws_endpoint,
    })
}

/// Reads `DevToolsActivePort` from the profile (written once the browser listens).
async fn wait_for_ws_endpoint(
    user_data_dir: &Path,
    child: &tokio::sync::Mutex<Option<Child>>,
    timeout_ms: u64,
) -> Result<String> {
    let file = user_data_dir.join("DevToolsActivePort");
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if let Ok(text) = tokio::fs::read_to_string(&file).await {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                return Ok(format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()));
            }
        }
        if let Some(c) = child.lock().await.as_mut()
            && let Ok(Some(status)) = c.try_wait()
        {
            return Err(Error::Launch(format!("browser exited early ({status})")));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(Error::Timeout {
                ms: timeout_ms,
                action: "waiting for DevToolsActivePort".into(),
            });
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(unix)]
mod unix_pipes {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

    use tokio::net::unix::pipe;

    use crate::domain::error::{Error, Result};

    /// Pipes wired to the child's fd 3 (it reads) and fd 4 (it writes).
    pub struct Pipes {
        parent_read: OwnedFd,
        parent_write: OwnedFd,
        _child_read: OwnedFd,
        _child_write: OwnedFd,
    }

    fn pipe_cloexec() -> std::io::Result<(OwnedFd, OwnedFd)> {
        let mut fds: [RawFd; 2] = [0; 2];
        // SAFETY: fds is a valid 2-element buffer.
        #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
        let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
        #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "freebsd")))]
        let rc = unsafe {
            let rc = libc::pipe(fds.as_mut_ptr());
            if rc == 0 {
                libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
                libc::fcntl(fds[1], libc::F_SETFD, libc::FD_CLOEXEC);
            }
            rc
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: pipe() returned two fresh descriptors we now own.
        Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
    }

    pub fn attach(cmd: &mut tokio::process::Command) -> Result<Pipes> {
        let map = |e: std::io::Error| Error::Launch(format!("cannot create pipes: {e}"));
        let (child_read, parent_write) = pipe_cloexec().map_err(map)?;
        let (parent_read, child_write) = pipe_cloexec().map_err(map)?;
        let (cr, cw) = (child_read.as_raw_fd(), child_write.as_raw_fd());
        // SAFETY: only async-signal-safe libc calls between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                let r = libc::fcntl(cr, libc::F_DUPFD, 10);
                let w = libc::fcntl(cw, libc::F_DUPFD, 10);
                if r < 0 || w < 0 || libc::dup2(r, 3) < 0 || libc::dup2(w, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                libc::close(r);
                libc::close(w);
                Ok(())
            });
        }
        Ok(Pipes {
            parent_read,
            parent_write,
            _child_read: child_read,
            _child_write: child_write,
        })
    }

    impl Pipes {
        /// Drops the child ends (after spawn) and wraps the parent ends.
        pub fn into_parent(self) -> Result<(pipe::Receiver, pipe::Sender)> {
            let map = |e: std::io::Error| Error::Launch(format!("cannot open pipes: {e}"));
            let Pipes {
                parent_read,
                parent_write,
                ..
            } = self;
            let reader = pipe::Receiver::from_owned_fd(parent_read).map_err(map)?;
            let writer = pipe::Sender::from_owned_fd(parent_write).map_err(map)?;
            Ok((reader, writer))
        }
    }
}
