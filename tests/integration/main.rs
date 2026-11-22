//! Integration tests against a real Chromium found by discovery.
//! No transport mocks: these talk to the browser over the real pipe/WebSocket.

mod aria;
mod assertions;
mod clock;
mod context;
mod context_events;
mod coverage;
mod debugger;
mod dragdrop;
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

pub fn full_chromium() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from("/opt/pw-browsers/chromium-1243/chrome-linux64/chrome");
    p.exists().then_some(p)
}
