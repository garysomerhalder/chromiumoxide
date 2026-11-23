//! adb port: the adb server's host protocol (Playwright's `backendAdb`).

use std::io;
use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;

use crate::domain::error::Result;
use crate::ports::CdpSession;

/// Where the adb server listens (default `127.0.0.1:5037`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbAddress {
    pub host: String,
    pub port: u16,
}

impl Default for AdbAddress {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 5037,
        }
    }
}

/// An open connection to a device service (`device.open(command)`).
pub trait AdbSocket: Send + Sync {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'static, io::Result<()>>;
    /// The bytes the service sends, until it closes. Can be taken once.
    fn data(&self) -> BoxStream<'static, Vec<u8>>;
    fn close(&self);
}

/// Talks to an adb server.
pub trait AdbClient: Send + Sync {
    /// `host:devices`: the raw device list.
    fn devices(&self, server: &AdbAddress) -> BoxFuture<'static, io::Result<String>>;
    /// A device command (`shell:…` and others): its whole output.
    fn run(
        &self,
        server: &AdbAddress,
        serial: &str,
        command: &str,
        input: Option<Vec<u8>>,
    ) -> BoxFuture<'static, io::Result<Vec<u8>>>;
    /// `sync:` push of `data` to `path` with `mode`.
    fn push(
        &self,
        server: &AdbAddress,
        serial: &str,
        data: Vec<u8>,
        path: &str,
        mode: u32,
    ) -> BoxFuture<'static, io::Result<()>>;
    /// A raw connection to a device service.
    fn open(
        &self,
        server: &AdbAddress,
        serial: &str,
        service: &str,
    ) -> BoxFuture<'static, io::Result<Box<dyn AdbSocket>>>;
    /// The CDP connection behind a DevTools socket on the device
    /// (`localabstract:<socket_name>`, WebSocket `/devtools/browser`).
    fn devtools(
        &self,
        server: &AdbAddress,
        serial: &str,
        socket_name: &str,
    ) -> BoxFuture<'static, Result<Arc<dyn CdpSession>>>;
}
