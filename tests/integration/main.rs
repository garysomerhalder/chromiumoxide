//! Integration tests against a real Chromium found by discovery.
//! No transport mocks: these talk to the browser over the real pipe/WebSocket.

mod assertions;
mod context;
mod events;
mod extras;
mod extras2;
mod frames;
mod handles;
mod handles2;
mod har;
mod input;
mod launch;
mod locator;
mod navigation;
mod network;
mod page;
mod pdf;
mod pixel_parity;
mod popups;
mod protocol;
mod routing;
mod screenshot;
mod scripts;
mod server;
mod shortcuts;
mod storage;
mod tracing;
mod video;
mod workers;

pub fn full_chromium() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from("/opt/pw-browsers/chromium-1243/chrome-linux64/chrome");
    p.exists().then_some(p)
}
