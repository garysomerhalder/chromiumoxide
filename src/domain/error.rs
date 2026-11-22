//! The error taxonomy.

use std::path::PathBuf;

use crate::domain::discovery::Source;

/// Errors raised by chromiumoxide.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// A browser path that was asked for explicitly does not exist.
    #[error("browser executable from {source:?} not found at {path}")]
    ExecutableMissing { path: PathBuf, source: Source },
    /// No browser was found anywhere.
    #[error("no Chromium found; looked in: {}", tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))]
    BrowserNotFound { tried: Vec<PathBuf> },
}

/// Result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;
