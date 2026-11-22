use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::domain::discovery::Os;
use crate::ports::{Environment, FileSystem};

/// A file system made of a set of file paths (directories are implied).
#[derive(Default)]
pub struct MemoryFileSystem {
    files: Mutex<BTreeSet<PathBuf>>,
    dirs: Mutex<BTreeSet<PathBuf>>,
    counter: Mutex<u32>,
}

impl MemoryFileSystem {
    pub fn with_files<I: IntoIterator<Item = S>, S: AsRef<str>>(files: I) -> Self {
        let fs = Self::default();
        for f in files {
            fs.add_file(f.as_ref());
        }
        fs
    }

    pub fn add_file(&self, path: &str) {
        let path = PathBuf::from(path);
        let mut dirs = self.dirs.lock().unwrap();
        for a in path.ancestors().skip(1) {
            dirs.insert(a.to_path_buf());
        }
        self.files.lock().unwrap().insert(path);
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> bool {
        self.files.lock().unwrap().contains(path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.lock().unwrap().contains(path)
    }
    fn list_dir(&self, path: &Path) -> Vec<String> {
        let files = self.files.lock().unwrap();
        let dirs = self.dirs.lock().unwrap();
        let names: BTreeSet<String> = files
            .iter()
            .chain(dirs.iter())
            .filter(|p| p.parent() == Some(path))
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        names.into_iter().collect()
    }
    fn create_temp_dir(&self, prefix: &str) -> io::Result<PathBuf> {
        let mut c = self.counter.lock().unwrap();
        *c += 1;
        let p = PathBuf::from(format!("/tmp/{prefix}{c}"));
        self.dirs.lock().unwrap().insert(p.clone());
        Ok(p)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.files.lock().unwrap().retain(|f| f != path);
        Ok(())
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        self.dirs.lock().unwrap().retain(|d| !d.starts_with(path));
        self.files.lock().unwrap().retain(|f| !f.starts_with(path));
        Ok(())
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.is_file(path) {
            Ok(Vec::new())
        } else {
            Err(io::ErrorKind::NotFound.into())
        }
    }

    fn write(&self, path: &Path, _data: &[u8]) -> io::Result<()> {
        self.add_file(&path.to_string_lossy());
        Ok(())
    }
}

/// An environment backed by a map.
pub struct MapEnvironment {
    pub vars: BTreeMap<String, String>,
    pub home: Option<PathBuf>,
    pub os: Os,
}

impl MapEnvironment {
    pub fn linux(home: &str) -> Self {
        Self {
            vars: BTreeMap::new(),
            home: Some(home.into()),
            os: Os::Linux,
        }
    }
    pub fn set(mut self, k: &str, v: &str) -> Self {
        self.vars.insert(k.into(), v.into());
        self
    }
}

impl Environment for MapEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }
    fn home_dir(&self) -> Option<PathBuf> {
        self.home.clone()
    }
    fn os(&self) -> Os {
        self.os
    }
}

/// A video encoder that writes nothing (unit tests).
pub struct NullVideo;

struct NullSink;

impl crate::ports::VideoEncoder for NullVideo {
    fn start(
        &self,
        _output: &Path,
        _width: u32,
        _height: u32,
    ) -> futures::future::BoxFuture<'static, io::Result<Box<dyn crate::ports::VideoSink>>> {
        Box::pin(async { Ok(Box::new(NullSink) as Box<dyn crate::ports::VideoSink>) })
    }
}

impl crate::ports::VideoSink for NullSink {
    fn write_frame<'a>(
        &'a mut self,
        _jpeg: &'a [u8],
    ) -> futures::future::BoxFuture<'a, io::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn finish(self: Box<Self>) -> futures::future::BoxFuture<'static, io::Result<()>> {
        Box::pin(async { Ok(()) })
    }
}
