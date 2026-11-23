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
    BrowserLauncher, BrowserProcess, Channel, ElectronCommand, LaunchCommand, Launched,
    LaunchedElectron, Transport,
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
    #[cfg(not(unix))]
    if command.channel == Channel::Pipe {
        return Err(Error::Launch(
            "the pipe channel is only supported on unix; use Channel::WebSocket".into(),
        ));
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
        Channel::Pipe => unreachable!(),
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
