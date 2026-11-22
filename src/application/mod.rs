//! Application layer: use cases and the public Playwright-like API.

pub mod actions;
pub mod browser;
pub mod context;
pub mod discovery;
pub mod events;
pub mod frame;
pub mod handle;
pub mod input;
pub mod js;
pub mod locator;
pub mod navigation;
pub mod page;
pub mod page_init;
pub mod protocol;
pub mod screenshot;
pub mod scripts;
pub mod services;
pub(crate) mod shared;
pub mod shortcuts;
pub mod wait;
