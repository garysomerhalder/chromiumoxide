//! The error taxonomy.

use std::path::PathBuf;

use crate::domain::discovery::Source;

/// Errors raised by chromiumoxide.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A browser path that was asked for explicitly does not exist.
    #[error("browser executable from {origin:?} not found at {path}")]
    ExecutableMissing { path: PathBuf, origin: Source },
    /// No browser was found anywhere.
    #[error("no Chromium found; looked in: {}", tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    BrowserNotFound { tried: Vec<PathBuf> },
    /// The browser answered a protocol call with an error.
    #[error("protocol error ({method}): {message} (code {code})")]
    Protocol {
        method: String,
        code: i64,
        message: String,
    },
    /// The connection, browser, context or page went away.
    #[error("target closed: {0}")]
    TargetClosed(String),
    /// A wait did not finish in time.
    #[error("timeout {ms}ms exceeded while {action}")]
    Timeout { ms: u64, action: String },
    /// Transport or process level failure.
    #[error("transport error: {0}")]
    Transport(String),
    /// Failed to launch the browser process.
    #[error("failed to launch browser: {0}")]
    Launch(String),
    /// Navigation failed (net error, aborted, ...).
    #[error("navigation failed: {0}")]
    Navigation(String),
    /// JavaScript threw, or a value could not be (de)serialised.
    #[error("evaluation failed: {0}")]
    Evaluation(String),
    /// An option or argument is invalid.
    #[error("{0}")]
    InvalidArgument(String),
    /// Protocol payload could not be (de)serialised.
    #[error("serialization error: {0}")]
    Serde(String),
    /// I/O error.
    #[error("i/o error: {0}")]
    Io(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e.to_string())
    }
}

impl From<crate::domain::emulation::InvalidOptions> for Error {
    fn from(e: crate::domain::emulation::InvalidOptions) -> Self {
        Error::InvalidArgument(e.0)
    }
}

impl From<crate::domain::screenshot::ScreenshotError> for Error {
    fn from(e: crate::domain::screenshot::ScreenshotError) -> Self {
        Error::InvalidArgument(e.0)
    }
}

/// Result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
