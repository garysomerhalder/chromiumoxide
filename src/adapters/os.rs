//! Operating-system adapters: file system and environment.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::discovery::Os;
use crate::domain::source::{SourceLocation, backtrace_locations, resolve_frame_file};
use crate::ports::{Environment, FileSystem};

/// The real file system.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsFileSystem;

impl FileSystem for OsFileSystem {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn list_dir(&self, path: &Path) -> Vec<String> {
        std::fs::read_dir(path)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn create_temp_dir(&self, prefix: &str) -> io::Result<PathBuf> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let base = std::env::temp_dir();
        loop {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or_default();
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = base.join(format!("{prefix}{}-{n}-{nanos:x}", std::process::id()));
            match std::fs::create_dir(&dir) {
                Ok(()) => return Ok(dir),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        // Chromium may still be flushing files for a moment after exit.
        let mut last = Ok(());
        for _ in 0..10 {
            match std::fs::remove_dir_all(path) {
                Ok(()) => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => last = Err(e),
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        last
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, data)
    }
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsEnvironment;

impl Environment for OsEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }

    fn os(&self) -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    fn call_stack(&self) -> Vec<SourceLocation> {
        let text = std::backtrace::Backtrace::force_capture().to_string();
        let cwd = std::env::current_dir().ok();
        backtrace_locations(&text)
            .into_iter()
            .map(|mut loc| {
                loc.file = resolve_frame_file(&loc.file, cwd.as_deref());
                loc
            })
            .collect()
    }
}
