//! Process environment port.

use std::path::PathBuf;

use crate::domain::discovery::Os;
use crate::domain::source::SourceLocation;

/// Read-only view of the process environment.
pub trait Environment: Send + Sync {
    fn var(&self, name: &str) -> Option<String>;
    fn home_dir(&self) -> Option<PathBuf>;
    fn os(&self) -> Os;
    /// The source locations of the current thread's stack, innermost first,
    /// with absolute paths (empty without debug info).
    fn call_stack(&self) -> Vec<SourceLocation> {
        Vec::new()
    }
}
