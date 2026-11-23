//! Integration tests against a real Chromium found by discovery.
//! No transport mocks: these talk to the browser over the real pipe/WebSocket.

mod aria;
mod assertions;
mod backlog;
mod client_certs;
mod clock;
mod context;
mod context_events;
mod coverage;
mod debugger;
mod dragdrop;
mod electron;
mod events;
mod extras;
mod extras2;
mod extras3;
mod frames;
mod handles;
mod handles2;
mod har;
mod https;
mod input;
mod launch;
mod locator;
mod navigation;
mod network;
mod oopif;
mod page;
mod pdf;
mod pixel_parity;
mod popups;
mod protocol;
mod pw165;
mod request;
mod routing;
mod screencast;
mod screenshot;
mod scripts;
mod server;
mod shortcuts;
mod snapshots;
mod storage;
mod tracing;
mod video;
mod workers;

/// Where Playwright installs browsers: `PLAYWRIGHT_BROWSERS_PATH`, the
/// container image's `/opt/pw-browsers`, and the user cache (CI).
pub fn playwright_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(p) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        dirs.push(p.into());
    }
    dirs.push("/opt/pw-browsers".into());
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(home).join(".cache/ms-playwright"));
    }
    dirs
}

/// The newest `<prefix><revision>/<exe>` in the Playwright directories.
pub fn newest_playwright_install(prefix: &str, exe: &str) -> Option<std::path::PathBuf> {
    let mut found: Vec<(u32, std::path::PathBuf)> = playwright_dirs()
        .into_iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let rev = name.strip_prefix(prefix)?.parse().ok()?;
            let path = e.path().join(exe);
            path.exists().then_some((rev, path))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

/// The newest full Chromium Playwright installed (`chromium-<revision>`).
pub fn full_chromium() -> Option<std::path::PathBuf> {
    newest_playwright_install("chromium-", "chrome-linux64/chrome")
}
