//! The adb host protocol over TCP, as Playwright's `backendAdb` speaks it:
//! each request is a 4-hex-digit length and the text, answered `OKAY` or
//! `FAIL` + a length-prefixed message.

use std::io;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use futures::{FutureExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::adapters::cdp::Connection;
use crate::domain::error::{Error, Result};
use crate::ports::CdpSession;
use crate::ports::adb::{AdbAddress, AdbClient, AdbSocket};

/// The production adb client.
#[derive(Debug, Default, Clone, Copy)]
pub struct TcpAdb;

fn encode(message: &str) -> Vec<u8> {
    format!("{:04x}{message}", message.len()).into_bytes()
}

async fn status(stream: &mut TcpStream, what: &str) -> io::Result<()> {
    let mut status = [0u8; 4];
    stream.read_exact(&mut status).await?;
    if &status == b"OKAY" {
        return Ok(());
    }
    let mut message = String::new();
    let mut len = [0u8; 4];
    if stream.read_exact(&mut len).await.is_ok()
        && let Some(n) = std::str::from_utf8(&len)
            .ok()
            .and_then(|s| usize::from_str_radix(s, 16).ok())
    {
        let mut buf = vec![0u8; n];
        if stream.read_exact(&mut buf).await.is_ok() {
            message = String::from_utf8_lossy(&buf).into_owned();
        }
    }
    Err(io::Error::other(format!(
        "adb {what}: {} {message}",
        String::from_utf8_lossy(&status)
    )))
}

/// Connects, selects the device (`host:transport:<serial>`) and sends `command`.
async fn connect(
    server: &AdbAddress,
    serial: Option<&str>,
    command: &str,
) -> io::Result<TcpStream> {
    let mut stream = TcpStream::connect((server.host.as_str(), server.port))
        .await
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("adb server {}:{}: {e}", server.host, server.port),
            )
        })?;
    if let Some(serial) = serial {
        stream
            .write_all(&encode(&format!("host:transport:{serial}")))
            .await?;
        status(&mut stream, "host:transport").await?;
    }
    stream.write_all(&encode(command)).await?;
    status(&mut stream, command).await?;
    Ok(stream)
}

struct Socket {
    writer: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    data: Mutex<Option<BoxStream<'static, Vec<u8>>>>,
    reader: tokio::task::JoinHandle<()>,
}

impl AdbSocket for Socket {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'static, io::Result<()>> {
        let writer = self.writer.clone();
        async move { writer.lock().await.write_all(&data).await }.boxed()
    }

    fn data(&self) -> BoxStream<'static, Vec<u8>> {
        self.data
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| futures::stream::empty().boxed())
    }

    fn close(&self) {
        self.reader.abort();
        let writer = self.writer.clone();
        tokio::spawn(async move {
            let _ = writer.lock().await.shutdown().await;
        });
    }
}

impl AdbClient for TcpAdb {
    fn devices(&self, server: &AdbAddress) -> BoxFuture<'static, io::Result<String>> {
        let server = server.clone();
        async move {
            let mut stream = connect(&server, None, "host:devices").await?;
            let mut len = [0u8; 4];
            stream.read_exact(&mut len).await?;
            let n = usize::from_str_radix(&String::from_utf8_lossy(&len), 16)
                .map_err(io::Error::other)?;
            let mut buf = vec![0u8; n];
            stream.read_exact(&mut buf).await?;
            Ok(String::from_utf8_lossy(&buf).into_owned())
        }
        .boxed()
    }

    fn run(
        &self,
        server: &AdbAddress,
        serial: &str,
        command: &str,
        input: Option<Vec<u8>>,
    ) -> BoxFuture<'static, io::Result<Vec<u8>>> {
        let (server, serial, command) = (server.clone(), serial.to_owned(), command.to_owned());
        async move {
            let mut stream = connect(&server, Some(&serial), &command).await?;
            if let Some(input) = input {
                stream.write_all(&input).await?;
            }
            let mut out = Vec::new();
            if command.starts_with("shell:") {
                stream.read_to_end(&mut out).await?;
            } else {
                let mut len = [0u8; 4];
                stream.read_exact(&mut len).await?;
                let n = usize::from_str_radix(&String::from_utf8_lossy(&len), 16)
                    .map_err(io::Error::other)?;
                out = vec![0u8; n];
                stream.read_exact(&mut out).await?;
            }
            Ok(out)
        }
        .boxed()
    }

    fn push(
        &self,
        server: &AdbAddress,
        serial: &str,
        data: Vec<u8>,
        path: &str,
        mode: u32,
    ) -> BoxFuture<'static, io::Result<()>> {
        let (server, serial, path) = (server.clone(), serial.to_owned(), path.to_owned());
        async move {
            let mut stream = connect(&server, Some(&serial), "sync:").await?;
            let header = |id: &[u8], len: u32| {
                let mut h = id.to_vec();
                h.extend_from_slice(&len.to_le_bytes());
                h
            };
            let spec = format!("{path},{mode}");
            stream
                .write_all(&header(b"SEND", spec.len() as u32))
                .await?;
            stream.write_all(spec.as_bytes()).await?;
            for chunk in data.chunks(65535) {
                stream
                    .write_all(&header(b"DATA", chunk.len() as u32))
                    .await?;
                stream.write_all(chunk).await?;
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as u32)
                .unwrap_or_default();
            stream.write_all(&header(b"DONE", now)).await?;
            let mut reply = [0u8; 4];
            stream.read_exact(&mut reply).await?;
            if &reply != b"OKAY" {
                return Err(io::Error::other(format!(
                    "Could not push: {}",
                    String::from_utf8_lossy(&reply)
                )));
            }
            Ok(())
        }
        .boxed()
    }

    fn open(
        &self,
        server: &AdbAddress,
        serial: &str,
        service: &str,
    ) -> BoxFuture<'static, io::Result<Box<dyn AdbSocket>>> {
        let (server, serial, service) = (server.clone(), serial.to_owned(), service.to_owned());
        async move {
            let stream = connect(&server, Some(&serial), &service).await?;
            let (mut read, write) = stream.into_split();
            let (tx, rx) = futures::channel::mpsc::unbounded();
            let reader = tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                while let Ok(n) = read.read(&mut buf).await {
                    if n == 0 || tx.unbounded_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });
            Ok(Box::new(Socket {
                writer: Arc::new(tokio::sync::Mutex::new(write)),
                data: Mutex::new(Some(rx.boxed())),
                reader,
            }) as Box<dyn AdbSocket>)
        }
        .boxed()
    }

    fn devtools(
        &self,
        server: &AdbAddress,
        serial: &str,
        socket_name: &str,
    ) -> BoxFuture<'static, Result<Arc<dyn CdpSession>>> {
        let service = format!("localabstract:{socket_name}");
        let (server, serial) = (server.clone(), serial.to_owned());
        async move {
            let stream = connect(&server, Some(&serial), &service)
                .await
                .map_err(|e| Error::Transport(e.to_string()))?;
            let transport = crate::adapters::transport::websocket::over_stream(
                stream,
                "ws://localhost/devtools/browser",
            )
            .await?;
            Ok(Arc::new(Connection::spawn(transport)) as Arc<dyn CdpSession>)
        }
        .boxed()
    }
}
