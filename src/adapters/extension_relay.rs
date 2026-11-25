//! The relay the browser extension (`extension/`) connects to. Extensions
//! cannot listen for connections, so the library listens: the extension
//! learns the port from the relay's connect page (`/chromiumoxide-connect`)
//! when a tab opens it, and connects to `/extension`. That WebSocket then
//! carries the browser's protocol, relayed by the extension.

use std::time::Duration;

use async_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use crate::domain::error::{Error, Result};
use crate::ports::Transport;

/// The connect page's path.
pub const CONNECT_PATH: &str = "/chromiumoxide-connect";
/// The extension's WebSocket path.
pub const EXTENSION_PATH: &str = "/extension";

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>chromiumoxide</title>\
<body style=\"font:16px system-ui;margin:3em\"><h1>chromiumoxide</h1>\
<p>This tab tells the chromiumoxide extension where the library is listening. \
Once connected you can close it.</p>";

/// A listening relay: it serves the connect page from the start, and
/// hands over the extension's socket once it connects.
pub struct Listener {
    url: String,
    connected: tokio::sync::oneshot::Receiver<Transport>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Listens on `host:port` (default `127.0.0.1`, any free port).
pub async fn listen(host: Option<&str>, port: Option<u16>) -> Result<Listener> {
    let host = host.unwrap_or("127.0.0.1");
    let listener = TcpListener::bind((host, port.unwrap_or(0)))
        .await
        .map_err(|e| Error::Transport(format!("cannot listen on {host}: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Transport(e.to_string()))?
        .port();
    let shown = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let (tx, connected) = tokio::sync::oneshot::channel();
    // Each connection on its own task: browsers open idle preconnect
    // sockets, which must not hold up the extension's.
    let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
    let task = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Some(transport) = serve(stream).await {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(transport);
                    }
                }
            });
        }
    });
    Ok(Listener {
        url: format!("http://{shown}:{port}{CONNECT_PATH}"),
        connected,
        task,
    })
}

impl Listener {
    /// The page that tells the extension where to connect.
    pub fn connect_url(&self) -> &str {
        &self.url
    }

    /// Waits for the extension to connect (or `timeout`).
    pub async fn accept(mut self, timeout: Duration) -> Result<Transport> {
        match tokio::time::timeout(timeout, &mut self.connected).await {
            Ok(Ok(transport)) => Ok(transport),
            Ok(Err(_)) => Err(Error::Transport("the relay stopped listening".into())),
            Err(_) => Err(Error::Timeout {
                ms: timeout.as_millis() as u64,
                action: format!(
                    "waiting for the browser extension (open {} in the browser that has it)",
                    self.url
                ),
            }),
        }
    }
}

/// The extension's WebSocket, or `None` after answering anything else.
async fn serve(mut stream: TcpStream) -> Option<Transport> {
    let mut head = [0u8; 2048];
    let n = stream.peek(&mut head).await.ok()?;
    let head = String::from_utf8_lossy(&head[..n]).to_ascii_lowercase();
    let path = head
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    if head.contains("upgrade: websocket") && path == EXTENSION_PATH {
        let config = WebSocketConfig::default()
            .max_message_size(None)
            .max_frame_size(None);
        let ws = async_tungstenite::tokio::accept_async_with_config(stream, Some(config))
            .await
            .ok()?;
        return Some(crate::adapters::transport::websocket::wrap(ws));
    }
    let (status, body) = if path.starts_with(CONNECT_PATH) {
        ("200 OK", PAGE)
    } else {
        ("404 Not Found", "not found")
    };
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\n\
         cache-control: no-store\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
    let _ = stream.shutdown().await;
    None
}

/// Opens `url` in the default browser.
pub fn open_in_default_browser(url: &str) -> Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn()
        .map(drop)
        .map_err(|e| Error::Launch(format!("cannot open {url}: {e}")))
}
