//! Application layer: use cases and the public Playwright-like API.

pub mod actions;
pub mod browser;
pub mod context;
pub mod discovery;
pub mod js;
pub mod navigation;
pub mod page;
pub mod page_init;
pub mod protocol;
pub mod screenshot;
pub mod services;
pub(crate) mod shared;
pub mod wait;
