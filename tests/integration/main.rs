//! Integration tests against a real Chromium found by discovery.
//! No transport mocks: these talk to the browser over the real pipe/WebSocket.

mod context;
mod launch;
mod page;
mod pixel_parity;
mod protocol;
mod screenshot;
mod server;

pub fn full_chromium() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from("/opt/pw-browsers/chromium-1243/chrome-linux64/chrome");
    p.exists().then_some(p)
}
