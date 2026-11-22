//! chromiumoxide: a Chrome DevTools Protocol client with a Playwright-like API.
//!
//! See `docs/ARCHITECTURE.md`.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;

/// The generated Chrome DevTools Protocol types.
pub use chromiumoxide_cdp::cdp;
