//! File system port.

use std::io;
use std::path::{Path, PathBuf};

/// The file system operations the application needs.
pub trait FileSystem: Send + Sync {
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    /// Entry names of a directory; empty if it cannot be read.
    fn list_dir(&self, path: &Path) -> Vec<String>;
    /// Creates a new, unique, empty directory under the system temp dir.
    fn create_temp_dir(&self, prefix: &str) -> io::Result<PathBuf>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write(&self, path: &Path, data: &[u8]) -> io::Result<()>;
}
