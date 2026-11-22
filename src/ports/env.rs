//! Process environment port.

use std::path::PathBuf;

use crate::domain::discovery::Os;

/// Read-only view of the process environment.
pub trait Environment: Send + Sync {
    fn var(&self, name: &str) -> Option<String>;
    fn home_dir(&self) -> Option<PathBuf>;
    fn os(&self) -> Os;
}
