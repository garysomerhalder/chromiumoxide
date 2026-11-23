//! Finding Electron the way the `electron` npm package does
//! (`require('electron')`, which Playwright uses without `executablePath`).

use std::path::{Path, PathBuf};

/// `ELECTRON_OVERRIDE_DIST_PATH`, else `node_modules/electron` in the working
/// directory or a parent (its `path.txt` names the binary inside `dist/`).
pub fn find_executable() -> Option<PathBuf> {
    let binary = if cfg!(windows) {
        "electron.exe"
    } else if cfg!(target_os = "macos") {
        "Electron.app/Contents/MacOS/Electron"
    } else {
        "electron"
    };
    if let Some(dist) = std::env::var_os("ELECTRON_OVERRIDE_DIST_PATH") {
        let p = Path::new(&dist).join(binary);
        if p.exists() {
            return Some(p);
        }
    }
    let cwd = std::env::current_dir().ok()?;
    for dir in cwd.ancestors() {
        let package = dir.join("node_modules").join("electron");
        let Ok(name) = std::fs::read_to_string(package.join("path.txt")) else {
            continue;
        };
        let p = package.join("dist").join(name.trim());
        if p.exists() {
            return Some(p);
        }
    }
    None
}
